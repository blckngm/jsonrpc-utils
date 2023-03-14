use darling::{FromAttributes, FromMeta};
use proc_macro::TokenStream;
use proc_macro2::{Literal, Span};
use quote::{format_ident, quote};
use syn::{
    braced, parse::Parse, parse2, parse_macro_input, parse_quote, Attribute, AttributeArgs, FnArg,
    Ident, ImplItemMethod, ItemTrait, LitStr, Pat, PathArguments, Result, ReturnType, Token,
    TraitItem, TraitItemMethod, Type, Visibility,
};

#[proc_macro_attribute]
pub fn rpc(attrs: TokenStream, input: TokenStream) -> TokenStream {
    let attrs = parse_macro_input!(attrs as AttributeArgs);
    let attributes = match RpcAttributes::from_list(&attrs) {
        Ok(v) => v,
        Err(e) => {
            return TokenStream::from(e.write_errors());
        }
    };

    let mut item_trait = parse_macro_input!(input as ItemTrait);
    let vis = &item_trait.vis;
    let trait_name = &item_trait.ident;
    let trait_name_snake = to_snake_case(trait_name.to_string());
    let add_method_name = format_ident!("add_{}_methods", trait_name_snake);

    let to_schema_func = if attributes.openrpc {
        let to_schema_method_name = format_ident!("{}_schema", trait_name_snake);
        let method_objs = item_trait
            .items
            .iter_mut()
            .filter_map(|m| match m {
                TraitItem::Method(m) => Some(m),
                _ => None,
            })
            .map(|m| method_to_schema(m))
            .collect::<Result<Vec<_>>>();
        let method_objs = match method_objs {
            Ok(x) => x,
            Err(e) => return e.to_compile_error().into(),
        };
        let title = LitStr::new(&trait_name_snake, trait_name.span());
        quote!(
            #vis fn #to_schema_method_name() -> jsonrpc_utils::serde_json::Value {
                jsonrpc_utils::serde_json::json!({
                    "openrpc": "1.2.6",
                    "info": {
                        "title": #title,
                        "version": "1.0.0",
                    },
                    "methods": [#(#method_objs),*]
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
            TraitItem::Method(m) => Some(m),
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

        #vis fn #add_method_name(rpc: &mut jsonrpc_utils::jsonrpc_core::MetaIoHandler<Option<jsonrpc_utils::pub_sub::Session>>, rpc_impl: impl #trait_name + Clone + Send + Sync + 'static) {
            #(#add_methods)*
        }

        #to_schema_func
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
    items: Vec<ImplItemMethod>,
}

impl Parse for ImplMethods {
    fn parse(input: syn::parse::ParseStream) -> Result<Self> {
        let attributes = input.call(Attribute::parse_outer)?;
        input.parse::<Token![impl]>()?;
        let ident: Ident = input.parse()?;
        let mut items: Vec<ImplItemMethod> = Vec::new();
        let content;
        braced!(content in input);
        while !content.is_empty() {
            let vis: Visibility = content.parse()?;
            items.push({
                let m: TraitItemMethod = content.parse()?;
                ImplItemMethod {
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

fn rpc_client_impl(input: proc_macro2::TokenStream) -> syn::Result<proc_macro2::TokenStream> {
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

fn rewrite_method(m: &mut ImplItemMethod) -> Result<()> {
    let method_name = m.sig.ident.to_string();
    let method_name = Literal::string(&method_name);

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

fn method_to_schema(m: &TraitItemMethod) -> Result<proc_macro2::TokenStream> {
    let attrs = MethodAttributes::from_attributes(&m.attrs)?;
    if attrs.pub_sub.is_some() {
        return Ok(quote!());
    }
    let name = LitStr::new(&m.sig.ident.to_string(), m.sig.ident.span());
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
                        "schema": schemars::schema_for!(#ty),
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
                                Some(syn::GenericArgument::Type(t)) => t,
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
            "schema": schemars::schema_for!(#result_type),
        }
    };
    Ok(quote! {
        jsonrpc_utils::serde_json::json!({
            "name": #name,
            "params": [#(#params),*],
            "result": #result,
        })
    })
}

fn add_method(m: &mut TraitItemMethod) -> Result<proc_macro2::TokenStream> {
    let method_name = &m.sig.ident;

    let (rpc_attributes, other_attributes) = m
        .attrs
        .drain(..)
        .partition::<Vec<_>, _>(|a| a.path.is_ident("rpc"));
    let attributes = MethodAttributes::from_attributes(&rpc_attributes)?;
    m.attrs = other_attributes;

    let method_name_str = Literal::string(&method_name.to_string());
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

    Ok(if let Some(pub_sub) = attributes.pub_sub {
        let notify_method_lit = LitStr::new(&pub_sub.notify, Span::call_site());
        let unsubscribe_method_lit = LitStr::new(&pub_sub.unsubscribe, Span::call_site());
        quote! {
            jsonrpc_utils::pub_sub::add_pub_sub(rpc, #method_name_str, #notify_method_lit.into(), #unsubscribe_method_lit, {
                let rpc_impl = rpc_impl.clone();
                move |params: jsonrpc_utils::jsonrpc_core::Params| {
                    #parse_params
                    rpc_impl.#method_name(#params_names1).map_err(Into::into)
                }
            });
        }
    } else {
        quote! {
            rpc.add_method(#method_name_str, {
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

#[derive(FromMeta)]
struct RpcAttributes {
    #[darling(default)]
    openrpc: bool,
}

#[derive(FromAttributes)]
#[darling(attributes(rpc))]
struct MethodAttributes {
    pub_sub: Option<PubSubAttribute>,
}

#[derive(FromMeta)]
struct PubSubAttribute {
    notify: String,
    unsubscribe: String,
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
    use syn::{parse2, Expr, Stmt};

    use super::*;

    fn test_method(m: proc_macro2::TokenStream) -> Stmt {
        let output = add_method(&mut parse2(m).unwrap()).unwrap();
        println!("output: {}", output);
        parse2(output).unwrap()
    }

    fn test_to_schema(m: proc_macro2::TokenStream) -> Expr {
        let output = method_to_schema(&parse2(m).unwrap()).unwrap();
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

    #[test]
    fn test_methods_to_schema() {
        test_to_schema(quote!(
            async fn no_param(&self) -> Result<u64>;
        ));
        test_to_schema(quote!(
            async fn sleep(&self, x: u64) -> Result<u64>;
        ));
        test_to_schema(quote!(
            fn sleep(&self, a: i32, b: i32) -> Result<i32>;
        ));
        test_to_schema(quote!(
            fn sleep2(&self, a: Option<i32>, b: Option<i32>) -> Result<i32>;
        ));
    }
}
