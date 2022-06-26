use proc_macro::TokenStream;
use proc_macro2::{Literal, Span};
use quote::{format_ident, quote};
use syn::{
    braced, parse::Parse, parse2, parse_macro_input, parse_quote, Attribute, FnArg, Ident,
    ImplItemMethod, ItemTrait, LitStr, Pat, Result, Token, TraitItem, TraitItemMethod, Visibility,
};

#[proc_macro_attribute]
pub fn rpc(_attr: TokenStream, input: TokenStream) -> TokenStream {
    let mut item_trait = parse_macro_input!(input as ItemTrait);
    let vis = &item_trait.vis;
    let trait_name = &item_trait.ident;
    let add_method_name = format_ident!("add_{}_methods", to_snake_case(trait_name.to_string()));

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

        #vis fn #add_method_name(rpc: &mut MetaIoHandler<Option<Session>>, rpc_impl: impl #trait_name + Clone + Send + Sync + 'static) {
            #(#add_methods)*
        }
    };

    result.into()
}

#[proc_macro_attribute]
pub fn rpc_client(_attr: TokenStream, input: TokenStream) -> TokenStream {
    rpc_client_impl(input.into())
        .unwrap_or_else(|e| e.into_compile_error())
        .into()
}

/// Provide additional information about a pub/sub method.
#[proc_macro_attribute]
pub fn pub_sub(_attr: TokenStream, input: TokenStream) -> TokenStream {
    input
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
                .rpc(#method_name, &serde_json::value::to_raw_value(&(#params_names))?)
                .await
                .context(#method_name)?;
            serde_json::from_value(result).context(#method_name)
        });
    } else {
        m.block = parse_quote!({
            let result = self
                .inner
                .rpc(#method_name, &serde_json::value::to_raw_value(&(#params_names))?)
                .context(#method_name)?;
            serde_json::from_value(result).context(#method_name)
        });
    }
    Ok(())
}

fn add_method(m: &mut TraitItemMethod) -> Result<proc_macro2::TokenStream> {
    let method_name = &m.sig.ident;

    let pub_sub_attribute = get_pub_sub_attribute(&mut m.attrs)?;

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
    let params_names = quote!(#(#params_names ,)*);
    let params_tys = quote!(#(#params_tys ,)*);
    let result = if m.sig.asyncness.is_some() {
        quote!(rpc_impl.#method_name(#params_names).await)
    } else {
        quote!(rpc_impl.#method_name(#params_names))
    };

    Ok(if let Some(pub_sub) = pub_sub_attribute {
        let notify_method_lit = LitStr::new(&pub_sub.notify_method, Span::call_site());
        let unsubscribe_method_lit = LitStr::new(&pub_sub.unsubscribe_method, Span::call_site());
        quote! {
            jsonrpc_utils::pub_sub::add_pub_sub(rpc, #method_name_str, #notify_method_lit.into(), #unsubscribe_method_lit, {
                let rpc_impl = rpc_impl.clone();
                move |params: jsonrpc_core::Params| {
                    let (#params_names): (#params_tys) = params.parse()?;
                    rpc_impl.#method_name(#params_names)
                }
            });
        }
    } else {
        quote! {
            rpc.add_method(#method_name_str, {
                let rpc_impl = rpc_impl.clone();
                move |params: jsonrpc_core::Params| {
                    let rpc_impl = rpc_impl.clone();
                    async move {
                        let (#params_names): (#params_tys) = params.parse()?;
                        jsonrpc_core::serde_json::to_value(#result?).map_err(|_| jsonrpc_core::Error::internal_error())
                    }
                }
            });
        }
    })
}

struct PubSubAttribute {
    notify_method: String,
    unsubscribe_method: String,
}

impl Parse for PubSubAttribute {
    fn parse(input: syn::parse::ParseStream) -> Result<Self> {
        let mut result = PubSubAttribute {
            notify_method: String::new(),
            unsubscribe_method: String::new(),
        };
        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            if ident == "notify" {
                let value: LitStr = input.parse()?;
                result.notify_method = value.value();
            } else if ident == "unsubscribe" {
                let value: LitStr = input.parse()?;
                result.unsubscribe_method = value.value();
            } else {
                return Err(syn::Error::new(
                    ident.span(),
                    "expected `notify` or `unsubscribe`",
                ));
            }
            if !input.is_empty() {
                input.parse::<Token![,]>()?;
            }
        }
        if result.notify_method.is_empty() {
            return Err(input.error(r#"expected `notify = "notify_method_name"`"#));
        }
        if result.unsubscribe_method.is_empty() {
            return Err(input.error(r#"expected `unsubscribe = "unsubscribe_method_name"`"#));
        }
        Ok(result)
    }
}

fn get_pub_sub_attribute(attrs: &mut Vec<Attribute>) -> Result<Option<PubSubAttribute>> {
    let mut r = None;
    for a in attrs {
        if a.path.is_ident("pub_sub") {
            if r.is_some() {
                return Err(syn::Error::new_spanned(a, "duplicated pub_sub attribute"));
            }
            r = Some(a.parse_args()?)
        }
    }
    Ok(r)
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
        parse2(output).unwrap()
    }

    #[test]
    fn test_methods() {
        test_method(quote!(
            async fn sleep(&self, x: u64) -> Result<u64>;
        ));
        test_method(quote!(
            fn sleep(&self, a: i32, b: i32) -> Result<i32>;
        ));
        test_method(quote!(
            #[pub_sub(notify = "subscription", unsubscribe = "unsubscribe")]
            fn subscribe(&self, a: i32, b: i32) -> Result<S>;
        ));
    }
}
