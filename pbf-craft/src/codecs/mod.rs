//! Internal PBF codecs: blob framing, field decoding and block building/decoration.
//!
//! These modules are private; public API consumers interact with them through the
//! [`crate::readers`] and [`crate::writers`] modules.

pub mod blob;
pub mod block_builder;
pub mod block_decorators;
pub mod field;
