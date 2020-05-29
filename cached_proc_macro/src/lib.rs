use darling::FromMeta;
use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, parse_str, AttributeArgs, Block, FnArg, Ident, ItemFn, Pat, ReturnType, Type,
};

#[derive(FromMeta)]
struct MacroArgs {
    #[darling(default)]
    name: Option<String>,
    #[darling(default)]
    unbound: bool,
    #[darling(default)]
    size: Option<usize>,
    #[darling(default)]
    time: Option<u64>,
    #[darling(default)]
    key: Option<String>,
    #[darling(default)]
    convert: Option<String>,
    #[darling(default)]
    result: bool,
    #[darling(default)]
    option: bool,
}

#[proc_macro_attribute]
pub fn cached(args: TokenStream, input: TokenStream) -> TokenStream {
    let attr_args = parse_macro_input!(args as AttributeArgs);
    let args = match MacroArgs::from_list(&attr_args) {
        Ok(v) => v,
        Err(e) => {
            return TokenStream::from(e.write_errors());
        }
    };
    let input = parse_macro_input!(input as ItemFn);

    // pull out the parts of the input
    let _attributes = input.attrs;
    let visibility = input.vis;
    let signature = input.sig;
    let body = input.block;

    // pull out the parts of the function signature
    let fn_ident = signature.ident.clone();
    let inputs = signature.inputs.clone();
    let output = signature.output.clone();

    // pull out the names and types of the function inputs
    let input_tys = inputs
        .iter()
        .map(|input| match input {
            FnArg::Receiver(_) => panic!("methods (functions taking 'self') are not supported"),
            FnArg::Typed(pat_type) => pat_type.ty.clone(),
        })
        .collect::<Vec<Box<Type>>>();

    let input_names = inputs
        .iter()
        .map(|input| match input {
            FnArg::Receiver(_) => panic!("methods (functions taking 'self') are not supported"),
            FnArg::Typed(pat_type) => pat_type.pat.clone(),
        })
        .collect::<Vec<Box<Pat>>>();

    // pull out the output type
    let output_ty = match &output {
        ReturnType::Default => quote! {()},
        ReturnType::Type(_, ty) => quote! {#ty},
    };

    // make the cache identifier
    let cache_ident = match args.name {
        Some(name) => Ident::new(&name, fn_ident.span()),
        None => Ident::new(&fn_ident.to_string().to_uppercase(), fn_ident.span()),
    };

    // make the cache key type and block that converts the inputs into the key type
    let (cache_key_ty, key_convert_block) = match (&args.key, &args.convert) {
        (Some(key_str), Some(convert_str)) => {
            let cache_key_ty = parse_str::<Type>(key_str).expect("unable to parse cache key type");

            let key_convert_block =
                parse_str::<Block>(convert_str).expect("unable to parse key convert block");

            (quote! {#cache_key_ty}, quote! {#key_convert_block})
        }
        (None, None) => (
            quote! {(#(#input_tys),*)},
            quote! {(#(#input_names.clone()),*)},
        ),
        (_, _) => panic!("key and convert arguments must be used together or not at all"),
    };

    // make the cache type and create statement
    let (cache_ty, cache_create) = match (&args.unbound, &args.size, &args.time) {
        (true, None, None) => {
            let cache_ty = quote! {cached::UnboundCache<#cache_key_ty, #output_ty>};
            let cache_create = quote! {cached::UnboundCache::new()};
            (cache_ty, cache_create)
        }
        (false, Some(size), None) => {
            let cache_ty = quote! {cached::SizedCache<#cache_key_ty, #output_ty>};
            let cache_create = quote! {cached::SizedCache::with_size(#size)};
            (cache_ty, cache_create)
        }
        (false, None, Some(time)) => {
            let cache_ty = quote! {cached::TimedCache<#cache_key_ty, #output_ty>};
            let cache_create = quote! {cached::TimedCache::with_lifespan(#time)};
            (cache_ty, cache_create)
        }
        (false, None, None) => {
            let cache_ty = quote! {cached::UnboundCache<#cache_key_ty, #output_ty>};
            let cache_create = quote! {cached::UnboundCache::new()};
            (cache_ty, cache_create)
        }
        _ => panic!("cache types (unbound, size, or time) are mutually exclusive"),
    };

    // make the set cache block
    let set_cache_block = match (&args.result, &args.option) {
        (false, false) => quote! { cache.cache_set(key, result.clone()); },
        (true, false) => quote! {
            match result.clone() {
                Ok(result) => cache.cache_set(key, Ok(result)),
                _ => {},
            }
        },
        (false, true) => quote! {
            match result.clone() {
                Some(result) => cache.cache_set(key, Some(result)),
                _ => {},
            }
        },
        _ => panic!("the result and option attributes are mutually exclusive"),
    };

    // put it all together
    let expanded = quote! {
        #visibility static #cache_ident: once_cell::sync::Lazy<std::sync::Mutex<#cache_ty>> = once_cell::sync::Lazy::new(|| std::sync::Mutex::new(#cache_create));
        #visibility #signature {
            use cached::Cached;
            let key = #key_convert_block;
            {
                // check if the result is cached
                let mut cache = #cache_ident.lock().unwrap();
                if let Some(result) = cache.cache_get(&key) {
                    return result.clone();
                }
            }

            // run the function and cache the result
            fn inner(#inputs) #output #body;
            let result = inner(#(#input_names),*);

            let mut cache = #cache_ident.lock().unwrap();
            // cache.cache_set(key, result.clone());
            #set_cache_block

            result
        }
    };

    expanded.into()
}
