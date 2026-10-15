//! A host procedural macro used by the target fixture.

extern crate proc_macro;

/// Emit a string literal at compile time.
#[proc_macro]
pub fn message(_: proc_macro::TokenStream) -> proc_macro::TokenStream {
    "\"macro-value\"".parse().expect("static token stream")
}