//! Implementation of the `collections` module.
//!
//! Exposes the most commonly used container datatypes:
//! - `deque` — a double-ended queue ([`Type::Deque`])
//!
//! Types are exposed as callable `Value::Builtin(Builtins::Type(...))` module
//! attributes; their construction and behavior live in the corresponding
//! runtime types under `crate::types`.

use crate::{
    builtins::Builtins,
    bytecode::VM,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    resource::{ResourceError, ResourceTracker},
    types::{Module, Type},
    value::Value,
};

/// Creates the `collections` module and allocates it on the heap.
///
/// Returns a `HeapId` pointing to the newly allocated module.
///
/// # Panics
///
/// Panics if the required strings have not been pre-interned during prepare phase.
pub fn create_module(vm: &mut VM<'_, impl ResourceTracker>) -> Result<HeapId, ResourceError> {
    let mut module = Module::new(StaticStrings::Collections);

    module.set_attr(StaticStrings::Deque, Value::Builtin(Builtins::Type(Type::Deque)), vm);

    vm.heap.allocate(HeapData::Module(module))
}
