use proc_macro::TokenStream;

// ===========================================================================
// Router macro
// ===========================================================================
mod router_macro;

#[proc_macro_attribute]
pub fn route(args: TokenStream, input: TokenStream) -> TokenStream {
    router_macro::route(args, input)
}