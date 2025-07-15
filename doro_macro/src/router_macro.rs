//! 注册路由的宏
//!
//! 将入参换成一个 json value，这样路由器就可以在调用时统一传入 json value。
//! 然后在函数体中拼接出从 json value 中取值的过程，保证能够取到宏展开之前的
//! 入参变量。
//!
//! 参数解析的格式
//! **第一种：**
//!
//!     {
//!         "name": "Executioner2",
//!         "age": 18,
//!     }
//!
//!     #[route(code = 0)]
//!     async fn add_user(
//!        #[param] name: String,
//!        #[param] age: u32,
//!     ) {}
//!
//! **第二种:**
//!
//!     {
//!         "name": "Executioner2",
//!         "age": 18,
//!     }
//!
//!     struct User {
//!         name: String,
//!         age: u32,
//!     }
//!
//!     #[route(code = 0)]
//!     async fn add_user(
//!        #[body] user: User,
//!     ) {}
//!
//! **第三种:**
//!
//!     struct Wife {
//!         name: String,
//!         age: u32,
//!         address: Vec<String>,
//!     }
//!
//!     {
//!         "name": "Executioner2",
//!         "age": 18,
//!         "wife": {
//!             "name": "Mia",
//!             "age": 18,
//!             "address": ["Earth", "China"]
//!         }
//!     }
//!
//!     #[route(code = 0)]
//!     async fn add_user(
//!        #[param] name: String,
//!        #[param] age: u32,
//!        #[body] wife: Wife,
//!     ) {}
//!
//! 其中 `#[param]` 直接从 json 数据结构中取得相同 key 名的值。除非指定为 Option 类型，否则 json 数据
//! 结构中必须存在对应 key 名的值。`#[body]` 则从 json 数据结构中取得整个结构体的值。两者可以混合使用。
//! 默认为 `#[body]`，因此可以省略。
use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use std::mem;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{parse_macro_input, Meta, Token};
use syn::{parse_quote, Expr, ExprLit, FnArg, Result};

#[derive(Debug)]
struct Attributes {
    args: Punctuated<Meta, Token![,]>,
}

impl Parse for Attributes {
    fn parse(input: ParseStream) -> Result<Self> {
        Ok(Attributes {
            args: Punctuated::parse_terminated(&input)?,
        })
    }
}

pub fn route(args: TokenStream, input: TokenStream) -> TokenStream {
    let attrs = parse_macro_input!(args as Attributes);
    let mut item_fn = parse_macro_input!(input as syn::ItemFn);
    match regsiter_route(&attrs, &mut item_fn) {
        Ok(ts) => ts.into(),
        Err(e) => {
            let mut ts = e.to_compile_error();
            ts.extend(item_fn.to_token_stream());
            ts.into()
        }
    }
}

fn parse_code(attrs: &Attributes, item_fn: &mut syn::ItemFn) -> Result<u32> {
    let mut code = None;
    for attr in &attrs.args {
        if let Meta::NameValue(meta) = attr {
            if meta.path.is_ident("code") {
                if let Expr::Lit(ExprLit {
                                     lit: syn::Lit::Int(el),
                                     ..
                                 }) = &meta.value
                {
                    if code.is_some() {
                        return Err(syn::Error::new_spanned(meta, "duplicate code attribute"));
                    }
                    code = Some(el.base10_parse::<u32>()?);
                }
            }
        }
    }

    code.ok_or(syn::Error::new_spanned(item_fn, "missing code attribute"))
}

fn get_attrs_from_fn_arg(fn_arg: &FnArg) -> Result<&Vec<syn::Attribute>> {
    match fn_arg {
        FnArg::Receiver(r) => Err(syn::Error::new_spanned(r, "unsupported receiver")),
        FnArg::Typed(pat_ty) => Ok(&pat_ty.attrs),
    }
}

fn get_ty_from_fn_arg(fn_arg: &FnArg) -> Result<&syn::Type> {
    match fn_arg {
        FnArg::Receiver(r) => Err(syn::Error::new_spanned(r, "unsupported receiver")),
        FnArg::Typed(pat_ty) => Ok(&pat_ty.ty),
    }
}

fn get_mut_fn_arg(fn_arg: &FnArg) -> Result<&Option<Token![mut]>> {
    match fn_arg {
        FnArg::Receiver(r) => Err(syn::Error::new_spanned(r, "unsupported receiver")),
        FnArg::Typed(pat_ty) => {
            if let syn::Pat::Ident(ident) = &*pat_ty.pat {
                Ok(&ident.mutability)
            } else {
                Err(syn::Error::new_spanned(pat_ty, "unsupported pattern"))
            }
        }
    }
}

fn get_ident_from_fn_arg(fn_arg: &FnArg) -> Result<&syn::Ident> {
    match fn_arg {
        FnArg::Receiver(r) => Err(syn::Error::new_spanned(r, "unsupported receiver")),
        FnArg::Typed(pat_ty) => {
            if let syn::Pat::Ident(ident) = &*pat_ty.pat {
                Ok(&ident.ident)
            } else {
                Err(syn::Error::new_spanned(pat_ty, "unsupported pattern"))
            }
        },
    }
}

fn is_option_ty_from_fn_arg(fn_arg: &FnArg) -> Result<bool> {
    let ty = get_ty_from_fn_arg(fn_arg)?;
    if let syn::Type::Path(path) = ty {
        let path = path_to_string(&path.path);
        return Ok(path == "Option" || path == "std::option::Option")
    }
    Ok(false)
}

fn has_param_attr(arg: &FnArg) -> Result<bool> {
    let attrs = get_attrs_from_fn_arg(arg)?;
    Ok(attrs.iter().find(|a| path_to_string(&a.path()) == "param").is_some())
}

fn replace_fn_args(
    args: &mut Punctuated<FnArg, Token![,]>,
) -> Option<Punctuated<FnArg, Token![,]>> {
    if args.is_empty() {
        return None;
    }
    let mut new_args = Punctuated::new();
    let arg = parse_quote! { mut arg: json_value::JsonValue };
    new_args.push(arg);
    mem::swap(&mut new_args, args);
    Some(new_args)
}

fn implant_json_params_map(
    stmts: Vec<syn::Stmt>,
    inputs: &Option<Punctuated<FnArg, Token![,]>>,
) -> Result<Vec<syn::Stmt>> {
    if inputs.is_none() {
        return Ok(stmts);
    }
    let inputs = inputs.as_ref().unwrap();

    // 只有一个值和有多个值的逻辑有点区别，一个值的情况下，需要判断是否有 #[param] 属性
    // 如果没有，那么可以认定为是 #[body]，则直接转为实体结构体。
    // 多个值的情况下，则需要遍历每个参数，判断是否有 #[param] 属性，如果有，则从 json 取值，
    // 如果没有，则认为是 #[body]，则直接转为实体结构体。

    let mut implants: Vec<syn::Stmt> = vec![];

    if inputs.len() == 1 && !has_param_attr(&inputs[0])? {
        let ident = get_ident_from_fn_arg(&inputs[0])?;
        let ty = get_ty_from_fn_arg(&inputs[0])?;
        let mut_token = get_mut_fn_arg(&inputs[0])?;
        let ident_str = ident.to_string();
        implants.push(parse_quote! {
            let #mut_token #ident: #ty = serde_json::from_value(arg).expect(&format!("Failed to parse {} from JSON", #ident_str));
        })
    } else {
        // 需要传入的必定是一个结构体
        implants.push(parse_quote!{
            let mut args: serde_json::Map<std::string::String, serde_json::Value> = serde_json::from_value(arg).expect("Failed to parse Map from JSON");
        });
        for input in inputs {
            let ident = get_ident_from_fn_arg(input)?;
            let ty = get_ty_from_fn_arg(input)?;
            let mut_token = get_mut_fn_arg(input)?;
            let ident_str = ident.to_string();
            let is_option = is_option_ty_from_fn_arg(input)?;
            if is_option {
                implants.push(parse_quote! {
                    let #mut_token #ident: #ty = {
                        match args.remove(#ident_str) {
                            None => None,
                            Some(arg) => serde_json::from_value(arg).expect(&format!("Failed to parse {} from JSON", #ident_str))
                        }
                    };
                })
            } else {
                implants.push(parse_quote! {
                    let #mut_token #ident: #ty = {
                        let arg = args.remove(#ident_str).expect(&format!("{} must not be None", #ident_str));
                        serde_json::from_value(arg).expect(&format!("Failed to parse {} from JSON", #ident_str))
                    };
                })
            }
        }
    }

    implants.extend(stmts);
    Ok(implants)
}

fn regsiter_route(
    attrs: &Attributes,
    item_fn: &mut syn::ItemFn,
) -> Result<proc_macro2::TokenStream> {
    // 异步检查，只支持异步函数
    if item_fn.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(item_fn, "not an async function"));
    }

    let code = parse_code(attrs, item_fn)?;

    // 拆分出组成部分，要改变签名和 body 部分
    let sig = &mut item_fn.sig;
    let block = &mut item_fn.block;

    let inputs = replace_fn_args(&mut sig.inputs);
    block.stmts = implant_json_params_map(block.stmts.to_vec(), &inputs)?;
    let gr = generate_registration(code, &sig.ident, inputs.is_some());

    let ts = quote! {
        #item_fn

        #gr
    };

    Ok(ts)
}

fn generate_registration(
    code: u32,
    fn_ident: &syn::Ident,
    has_args: bool,
) -> proc_macro2::TokenStream {
    // 这个 span 不知道选哪个位置好，先选 macro 自身的位置
    let register_fn = syn::Ident::new(
        &format!("__register_route_{code}"),
        proc_macro2::Span::call_site(),
    );
    quote! { register_route!(#register_fn, #code, #fn_ident, #has_args); }.into()
}

fn path_to_string(path: &syn::Path) -> String {
    let segments: Vec<_> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    segments.join("::")
}
