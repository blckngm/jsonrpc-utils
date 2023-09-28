use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote, ToTokens};
use syn::{
    braced, meta, parenthesized,
    parse::{Parse, ParseStream, Parser},
    parse2, parse_macro_input, parse_quote, Attribute, FnArg, GenericArgument, Ident, ImplItemFn,
    ItemTrait, LitStr, Pat, PathArguments, Result, ReturnType, Token, TraitItem, TraitItemFn, Type,
    Visibility,
};

#[proc_macro_attribute]
pub fn rpc(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as RpcArgs);

    let mut item_trait = parse_macro_input!(input as ItemTrait);
    let vis = &item_trait.vis;
    let trait_name = &item_trait.ident;
    let trait_name_snake = to_snake_case(trait_name.to_string());
    let add_method_name = format_ident!("add_{}_methods", trait_name_snake);

    let doc_func = if args.openrpc {
        let doc_func_name = format_ident!("{}_doc", trait_name_snake);
        let method_defs = item_trait
            .items
            .iter_mut()
            .filter_map(|m| match m {
                TraitItem::Fn(m) => Some(m),
                _ => None,
            })
            .map(|m| method_def(m))
            .collect::<Result<Vec<_>>>();
        let method_defs = match method_defs {
            Ok(x) => x,
            Err(e) => return e.to_compile_error().into(),
        };
        let title = &*trait_name_snake;
        quote!(
            /// Generate OpenRPC document for the RPC methods.
            #vis fn #doc_func_name() -> jsonrpc_utils::serde_json::Value {
                #[allow(unused)]
                use schemars::JsonSchema;

                let mut gen = schemars::gen::SchemaSettings::draft07().with(|s|
                    s.definitions_path = "#/components/schemas/".into()
                ).into_generator();

                let methods = jsonrpc_utils::serde_json::json!([#(#method_defs)*]);

                jsonrpc_utils::serde_json::json!({
                    "openrpc": "1.2.6",
                    "info": {
                        "title": #title,
                        "version": "1.0.0",
                    },
                    "methods": methods,
                    "components": {
                        "schemas": gen.take_definitions(),
                    }
                })
            }
        )
    } else {
        quote!()
    };

    let add_methods = item_trait
        .items
        .iter_mut()
        .filter_map(|m| match m {
            TraitItem::Fn(m) => Some(m),
            _ => None,
        })
        .map(add_method)
        .collect::<Result<Vec<_>>>();
    let add_methods = match add_methods {
        Ok(x) => x,
        Err(e) => return e.to_compile_error().into(),
    };

    let result = quote! {
        #item_trait

        /// Add RPC methods to the given `jsonrpc_utils::jsonrpc_core::MetaIoHandler`.
        #vis fn #add_method_name(rpc: &mut jsonrpc_utils::jsonrpc_core::MetaIoHandler<Option<jsonrpc_utils::pub_sub::Session>>, rpc_impl: impl #trait_name + Clone + Send + Sync + 'static) {
            #(#add_methods)*
        }

        #doc_func
    };

    result.into()
}

#[proc_macro_attribute]
pub fn rpc_client(_attr: TokenStream, input: TokenStream) -> TokenStream {
    rpc_client_impl(input.into())
        .unwrap_or_else(|e| e.into_compile_error())
        .into()
}

struct ImplMethods {
    attributes: Vec<Attribute>,
    ident: Ident,
    items: Vec<ImplItemFn>,
}

impl Parse for ImplMethods {
    fn parse(input: ParseStream) -> Result<Self> {
        let attributes = input.call(Attribute::parse_outer)?;
        input.parse::<Token![impl]>()?;
        let ident: Ident = input.parse()?;
        let mut items: Vec<ImplItemFn> = Vec::new();
        let content;
        braced!(content in input);
        while !content.is_empty() {
            let vis: Visibility = content.parse()?;
            items.push({
                let m: TraitItemFn = content.parse()?;
                ImplItemFn {
                    attrs: m.attrs,
                    vis,
                    defaultness: None,
                    sig: m.sig,
                    block: parse_quote!({}),
                }
            });
        }
        Ok(Self {
            attributes,
            ident,
            items,
        })
    }
}

fn rpc_client_impl(input: proc_macro2::TokenStream) -> Result<proc_macro2::TokenStream> {
    let mut impl_block: ImplMethods = parse2(input)?;

    for item in &mut impl_block.items {
        rewrite_method(item)?;
    }

    let attributes = impl_block.attributes;
    let ident = impl_block.ident;
    let items = impl_block.items;

    Ok(quote! {
        #(#attributes)*
        impl #ident {
            #(#items)*
        }
    })
}

fn rewrite_method(m: &mut ImplItemFn) -> Result<()> {
    let args = ClientMethodArgs::parse_attrs(&m.attrs)?;

    m.attrs.retain(|a| !a.path().is_ident("rpc"));

    let method_name = args.name.unwrap_or_else(|| m.sig.ident.to_string());

    let mut ident_counter = 0u32;
    let mut next_ident_counter = || {
        ident_counter += 1;
        ident_counter
    };

    let mut params_names = Vec::new();
    for arg in &mut m.sig.inputs {
        match arg {
            FnArg::Receiver(_) => {}
            FnArg::Typed(pat_type) => match &*pat_type.pat {
                Pat::Ident(ident) => {
                    params_names.push(ident.ident.clone());
                }
                _ => {
                    let ident = format_ident!("param_{}", next_ident_counter());
                    params_names.push(ident.clone());
                    pat_type.pat = parse_quote!(#ident);
                }
            },
        }
    }
    let params_names = quote!(#(#params_names ,)*);

    if m.sig.asyncness.is_some() {
        m.block = parse_quote!({
            let result = self
                .inner
                .rpc(#method_name, &jsonrpc_utils::serde_json::value::to_raw_value(&(#params_names))?)
                .await?;
            Ok(jsonrpc_utils::serde_json::from_value(result)?)
        });
    } else {
        m.block = parse_quote!({
            let result = self
                .inner
                .rpc(#method_name, &jsonrpc_utils::serde_json::value::to_raw_value(&(#params_names))?)?;
            Ok(jsonrpc_utils::serde_json::from_value(result)?)
        });
    }
    Ok(())
}

fn method_def(m: &TraitItemFn) -> Result<proc_macro2::TokenStream> {
    let doc_attrs = m
        .attrs
        .iter()
        .filter(|a| a.path().is_ident("doc"))
        .collect::<Vec<_>>();
    let doc = if !doc_attrs.is_empty() {
        let mut values = vec![];
        for a in doc_attrs {
            let v = &a.meta.require_name_value()?.value;
            values.push(parse2::<LitStr>(v.to_token_stream())?);
        }
        Some(values)
    } else {
        None
    };

    let description = if let Some(lines) = doc {
        let doc = lines
            .iter()
            .map(|l| l.value())
            .collect::<Vec<_>>()
            .join("\n");
        let doc = LitStr::new(&doc, Span::call_site());
        quote!( "description": #doc, )
    } else {
        quote!()
    };

    let attrs = MethodArgs::parse_attrs(&m.attrs)?;
    if attrs.pub_sub.is_some() {
        return Ok(quote!());
    }
    let name = attrs.name.unwrap_or_else(|| m.sig.ident.to_string());
    let params: Vec<_> = m
        .sig
        .inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Receiver(_) => None,
            FnArg::Typed(pat_type) => {
                let name = match &*pat_type.pat {
                    Pat::Ident(i) => LitStr::new(&i.ident.to_string(), i.ident.span()),
                    _ => LitStr::new("parameter", Span::call_site()),
                };
                let ty = &pat_type.ty;
                Some(quote! {
                    {
                        "name": #name,
                        "schema": gen.subschema_for::<#ty>(),
                    }
                })
            }
        })
        .collect();
    let result_type = match &m.sig.output {
        ReturnType::Default => todo!(),
        ReturnType::Type(_, t) => match &**t {
            Type::Path(tp) => {
                if tp.qself.is_none() && tp.path.segments.len() == 1 {
                    let seg = tp.path.segments.first().unwrap();
                    if seg.ident == "Result" {
                        match &seg.arguments {
                            PathArguments::AngleBracketed(ang) => match ang.args.first() {
                                Some(GenericArgument::Type(t)) => t,
                                _ => t,
                            },
                            _ => t,
                        }
                    } else {
                        t
                    }
                } else {
                    t
                }
            }
            _ => t,
        },
    };
    // TODO: more meaningful result name.
    let result = quote! {
        {
            "name": #name,
            "schema": gen.subschema_for::<#result_type>(),
        }
    };
    Ok(quote! {
        jsonrpc_utils::serde_json::json!({
            "name": #name,
            #description
            "params": [#(#params),*],
            "result": #result,
        }),
    })
}

fn add_method(m: &mut TraitItemFn) -> Result<proc_macro2::TokenStream> {
    let method_name = &m.sig.ident;

    let (rpc_attributes, other_attributes) = m
        .attrs
        .drain(..)
        .partition::<Vec<_>, _>(|a| a.path().is_ident("rpc"));
    let args = MethodArgs::parse_attrs(&rpc_attributes)?;
    m.attrs = other_attributes;

    let rpc_method_name = args.name.unwrap_or_else(|| method_name.to_string());

    let mut params_names = Vec::new();
    let mut params_tys = Vec::new();
    let mut ident_counter = 0u32;
    let mut next_ident_counter = || {
        ident_counter += 1;
        ident_counter
    };
    for arg in &m.sig.inputs {
        match arg {
            FnArg::Receiver(_) => {}
            FnArg::Typed(pat_type) => match &*pat_type.pat {
                Pat::Ident(ident) => {
                    params_names.push(ident.ident.clone());
                    params_tys.push(&*pat_type.ty);
                }
                _ => {
                    params_names.push(format_ident!("param_{}", next_ident_counter()));
                    params_tys.push(&*pat_type.ty);
                }
            },
        }
    }
    let no_params = params_names.is_empty();
    // Number of tailing optional parameters.
    let optional_params = params_tys
        .iter()
        .rev()
        .take_while(|t| match t {
            Type::Path(t) => t
                .path
                .segments
                .first()
                .map_or(false, |s| s.ident == "Option"),
            _ => false,
        })
        .count();
    let params_names1 = quote!(#(#params_names ,)*);
    let params_tys1 = quote!(#(#params_tys ,)*);
    let parse_params = if optional_params > 0 {
        let required_params = params_names.len() - optional_params;
        let mut parse_params = quote! {
            let mut arr = match params {
                jsonrpc_utils::jsonrpc_core::Params::Array(arr) => arr.into_iter(),
                jsonrpc_utils::jsonrpc_core::Params::None => Vec::new().into_iter(),
                _ => return Err(jsonrpc_utils::jsonrpc_core::Error::invalid_params("")),
            };
        };
        for i in 0..required_params {
            let p = &params_names[i];
            let ty = params_tys[i];
            parse_params.extend(quote! {
                let #p: #ty = jsonrpc_utils::serde_json::from_value(arr.next().ok_or_else(|| jsonrpc_utils::jsonrpc_core::Error::invalid_params(""))?).map_err(|_|
                    jsonrpc_utils::jsonrpc_core::Error::invalid_params("")
                )?;
            });
        }
        for i in required_params..params_names.len() {
            let p = &params_names[i];
            let ty = params_tys[i];
            parse_params.extend(quote! {
                let #p: #ty = match arr.next() {
                    Some(v) => jsonrpc_utils::serde_json::from_value(v).map_err(|_| jsonrpc_utils::jsonrpc_core::Error::invalid_params(""))?,
                    None => None,
                };
            });
        }
        parse_params
    } else if no_params {
        quote!(params.expect_no_params()?;)
    } else {
        quote!(let (#params_names1): (#params_tys1) = params.parse()?;)
    };
    let result = if m.sig.asyncness.is_some() {
        quote!(rpc_impl.#method_name(#params_names1).await)
    } else {
        quote!(rpc_impl.#method_name(#params_names1))
    };

    Ok(if let Some(pub_sub) = args.pub_sub {
        let notify_method_lit = &*pub_sub.notify;
        let unsubscribe_method_lit = &*pub_sub.unsubscribe;
        quote! {
            jsonrpc_utils::pub_sub::add_pub_sub(rpc, #rpc_method_name, #notify_method_lit.into(), #unsubscribe_method_lit, {
                let rpc_impl = rpc_impl.clone();
                move |params: jsonrpc_utils::jsonrpc_core::Params| {
                    #parse_params
                    rpc_impl.#method_name(#params_names1).map_err(Into::into)
                }
            });
        }
    } else {
        quote! {
            rpc.add_method(#rpc_method_name, {
                let rpc_impl = rpc_impl.clone();
                move |params: jsonrpc_utils::jsonrpc_core::Params| {
                    let rpc_impl = rpc_impl.clone();
                    async move {
                        #parse_params
                        jsonrpc_utils::serde_json::to_value(#result?).map_err(|_| jsonrpc_utils::jsonrpc_core::Error::internal_error())
                    }
                }
            });
        }
    })
}

struct RpcArgs {
    openrpc: bool,
}

impl Parse for RpcArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut openrpc = false;
        if !input.is_empty() {
            let parser = meta::parser(|m| {
                if m.path.is_ident("openrpc") {
                    openrpc = true;
                    Ok(())
                } else {
                    Err(m.error("unknown arg"))
                }
            });
            parser.parse2(input.parse()?)?;
        }
        Ok(Self { openrpc })
    }
}

struct ClientMethodArgs {
    name: Option<String>,
}

impl ClientMethodArgs {
    fn parse_attrs(attrs: &[Attribute]) -> Result<Self> {
        let mut name: Option<LitStr> = None;

        for a in attrs {
            if a.path().is_ident("rpc") {
                a.parse_nested_meta(|m| {
                    if m.path.is_ident("name") {
                        name = Some(m.value()?.parse()?);
                    } else {
                        return Err(m.error("unknown arg"));
                    }
                    Ok(())
                })?;
            }
        }

        let name = name.map(|n| n.value());
        Ok(Self { name })
    }
}

struct MethodArgs {
    pub_sub: Option<PubSubArgs>,
    name: Option<String>,
}

impl MethodArgs {
    fn parse_attrs(attrs: &[Attribute]) -> Result<Self> {
        let mut pub_sub: Option<PubSubArgs> = None;
        let mut name: Option<LitStr> = None;
        for a in attrs {
            if a.path().is_ident("rpc") {
                a.parse_nested_meta(|m| {
                    if m.path.is_ident("pub_sub") {
                        let content;
                        parenthesized!(content in m.input);
                        pub_sub = Some(content.parse()?);
                        Ok(())
                    } else if m.path.is_ident("name") {
                        name = Some(m.value()?.parse()?);
                        Ok(())
                    } else {
                        Err(m.error("unknown arg"))
                    }
                })?;
            }
        }
        let name = name.map(|n| n.value());
        Ok(Self { name, pub_sub })
    }
}

struct PubSubArgs {
    notify: String,
    unsubscribe: String,
}

impl Parse for PubSubArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut notify: Option<LitStr> = None;
        let mut unsubscribe: Option<LitStr> = None;

        let parser = meta::parser(|m| {
            if m.path.is_ident("notify") {
                notify = Some(m.value()?.parse()?);
            } else if m.path.is_ident("unsubscribe") {
                unsubscribe = Some(m.value()?.parse()?);
            } else {
                return Err(m.error("unknown arg"));
            }
            Ok(())
        });
        parser.parse2(input.parse()?)?;

        let notify = notify
            .ok_or_else(|| input.error("missing arg notify"))?
            .value();
        let unsubscribe = unsubscribe
            .ok_or_else(|| input.error("missing arg unsubscribe"))?
            .value();

        Ok(Self {
            notify,
            unsubscribe,
        })
    }
}

fn to_snake_case(ident: String) -> String {
    let mut result = String::with_capacity(ident.len());
    for c in ident.chars() {
        if c.is_ascii_uppercase() {
            if !result.is_empty() {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c)
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use syn::{parse2, Stmt};

    use super::*;

    fn test_method(m: proc_macro2::TokenStream) -> Stmt {
        let output = add_method(&mut parse2(m).unwrap()).unwrap();
        println!("output: {}", output);
        parse2(output).unwrap()
    }

    #[test]
    fn test_methods() {
        test_method(quote!(
            async fn no_param(&self) -> Result<u64>;
        ));
        test_method(quote!(
            async fn sleep(&self, x: u64) -> Result<u64>;
        ));
        test_method(quote!(
            #[rpc(name = "@sleep")]
            fn sleep(&self, a: i32, b: i32) -> Result<i32>;
        ));
        test_method(quote!(
            fn sleep2(&self, a: Option<i32>, b: Option<i32>) -> Result<i32>;
        ));
        test_method(quote!(
            #[rpc(pub_sub(notify = "subscription", unsubscribe = "unsubscribe"))]
            fn subscribe(&self, a: i32, b: i32) -> Result<S>;
        ));
    }
}
