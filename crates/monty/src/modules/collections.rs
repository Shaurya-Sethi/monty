//! Implementation of the `collections` module.
//!
//! Exposes the most commonly used container datatypes:
//! - `deque` — a double-ended queue ([`Type::Deque`])
//! - `defaultdict` — a dict with a default factory ([`Type::DefaultDict`])
//! - `Counter` — a dict for counting ([`Type::Counter`])
//! - `namedtuple` — a factory function producing named-tuple classes
//!
//! The container types are exposed as callable
//! `Value::Builtin(Builtins::Type(...))` module attributes; `namedtuple` is a
//! module-level function. Their behavior lives in the corresponding runtime
//! types under `crate::types`.

use crate::{
    args::ArgValues,
    builtins::Builtins,
    bytecode::VM,
    exception_private::RunResult,
    heap::{HeapData, HeapId},
    intern::StaticStrings,
    modules::ModuleFunctions,
    resource::{ResourceError, ResourceTracker},
    types::{Module, Type, namedtuple_class::make_namedtuple},
    value::Value,
};

/// Module-level functions of the `collections` module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, serde::Serialize, serde::Deserialize)]
#[strum(serialize_all = "lowercase")]
pub(crate) enum CollectionsFunctions {
    /// `collections.namedtuple(typename, field_names, *, rename, defaults, module)`.
    Namedtuple,
}

/// Dispatches a `collections` module function call.
pub fn call(vm: &mut VM<'_, impl ResourceTracker>, func: CollectionsFunctions, args: ArgValues) -> RunResult<Value> {
    match func {
        CollectionsFunctions::Namedtuple => make_namedtuple(vm, args),
    }
}

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
    module.set_attr(
        StaticStrings::Defaultdict,
        Value::Builtin(Builtins::Type(Type::DefaultDict)),
        vm,
    );
    module.set_attr(
        StaticStrings::Counter,
        Value::Builtin(Builtins::Type(Type::Counter)),
        vm,
    );
    module.set_attr(
        StaticStrings::Namedtuple,
        Value::ModuleFunction(ModuleFunctions::Collections(CollectionsFunctions::Namedtuple)),
        vm,
    );

    vm.heap.allocate(HeapData::Module(module))
}
