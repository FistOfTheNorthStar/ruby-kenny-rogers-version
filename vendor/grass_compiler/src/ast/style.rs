use codemap::{Span, Spanned};

use crate::{interner::InternedString, value::Value};

/// A style: `color: red`
#[derive(Clone, Debug)]
pub(crate) struct Style {
    pub property: InternedString,
    pub value: Box<Spanned<Value>>,
    pub declared_as_custom_property: bool,
    /// The declaration (from its property name), for source maps
    pub span: Span,
}
