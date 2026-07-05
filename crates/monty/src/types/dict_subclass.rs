use std::{cmp::Reverse, fmt::Write, mem};

use super::{Dict, LazyHeapSet, MontyIter, PyTrait, allocate_tuple};
use crate::{
    args::{ArgValues, KwargsValues},
    bytecode::{CallResult, VM},
    defer_drop, defer_drop_mut,
    exception_private::{ExcType, RunResult, SimpleException},
    heap::{DropWithHeap, HeapData, HeapGuard, HeapId, HeapItem, HeapRead, HeapReadOutput},
    intern::StaticStrings,
    resource::ResourceTracker,
    types::{
        List, Type,
        dict::{dict_merge_from_kwargs, dict_merge_from_value},
    },
    value::{EitherStr, VALUE_SIZE, Value},
};

/// A `dict` subclass that wraps a backing `dict` heap entry plus a small amount
/// of extra state. Covers `collections.defaultdict` and `collections.Counter`
/// in a single heap variant to keep central dispatch churn low.
///
/// The backing dict is a **separate** `HeapData::Dict` entry referenced by
/// `dict_id` (owned: one ref held for the wrapper's lifetime). Keeping it as its
/// own entry — rather than embedding a `Dict` field — lets `keys()`/`values()`/
/// `items()` views reference the real dict, and lets `__getitem__` (which takes
/// `&self`) still mutate the backing dict on a `defaultdict` miss, since the
/// backing dict is a distinct heap entry.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct DictSubclass {
    /// HeapId of the backing `HeapData::Dict`.
    dict_id: HeapId,
    /// Which subclass this is, plus any per-kind state.
    kind: DictSubclassKind,
}

/// The concrete `dict` subclass, with per-kind state.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum DictSubclassKind {
    /// `defaultdict` — `factory` is the `default_factory` (may be `Value::None`).
    DefaultDict { factory: Value },
    /// `Counter`.
    Counter,
}

impl DictSubclass {
    /// Returns the backing dict's HeapId.
    #[must_use]
    pub fn dict_id(&self) -> HeapId {
        self.dict_id
    }

    /// Returns a reference to the subclass kind.
    #[must_use]
    pub fn kind(&self) -> &DictSubclassKind {
        &self.kind
    }

    /// Implements the `defaultdict([default_factory[, ...]], **kwargs)` constructor.
    ///
    /// The first positional argument (if any) is the `default_factory`, which
    /// must be callable or `None`; the remaining positional (an optional
    /// mapping/iterable) and keyword arguments initialize the dict exactly like
    /// `dict(...)`.
    pub fn init_defaultdict(vm: &mut VM<'_, impl ResourceTracker>, args: ArgValues) -> RunResult<Value> {
        let (pos, kwargs) = args.into_parts();
        let mut pos: Vec<Value> = pos.collect();

        // First positional is the factory; validate callable-or-None.
        let factory = if pos.is_empty() { Value::None } else { pos.remove(0) };
        if !matches!(factory, Value::None) && !is_callable(&factory, vm) {
            factory.drop_with_heap(vm);
            pos.drop_with_heap(vm);
            kwargs.drop_with_heap(vm);
            return Err(SimpleException::new_msg(ExcType::TypeError, "first argument must be callable or None").into());
        }
        if pos.len() > 1 {
            // The excess positionals would be handed to `dict(...)`, so CPython
            // surfaces the *dict* arity error (e.g. "dict expected at most 1
            // argument, got 2"), counting only the non-factory positionals.
            let got = pos.len();
            factory.drop_with_heap(vm);
            pos.drop_with_heap(vm);
            kwargs.drop_with_heap(vm);
            return Err(ExcType::type_error_at_most("dict", 1, got));
        }
        let source = pos.into_iter().next();

        // Build the backing dict off-heap (via the scoped guard pattern), then
        // allocate it and wrap it in the DictSubclass.
        let mut dict_guard = HeapGuard::new(Dict::new(), vm);
        {
            let (dict, vm) = dict_guard.as_parts_mut();
            if let Some(source) = source {
                dict_merge_from_value(dict, source, vm)?;
            }
            dict_merge_from_kwargs(dict, kwargs, vm)?;
        }
        let dict = dict_guard.into_inner();
        Self::finish(vm, dict, DictSubclassKind::DefaultDict { factory })
    }

    /// Implements the `Counter([iterable_or_mapping], **kwargs)` constructor,
    /// tallying counts from the source and keyword arguments.
    pub fn init_counter(vm: &mut VM<'_, impl ResourceTracker>, args: ArgValues) -> RunResult<Value> {
        let (pos, kwargs) = args.into_parts();
        let pos: Vec<Value> = pos.collect();

        if pos.len() > 1 {
            // CPython's C `Counter.__init__` reports a clinic-style range error
            // counting the implicit `self` (min 1, max 2).
            let actual = pos.len() + 1;
            pos.drop_with_heap(vm);
            kwargs.drop_with_heap(vm);
            return Err(ExcType::type_error_too_many_positional_range(
                "Counter.__init__",
                1,
                2,
                actual,
                0,
            ));
        }
        let source = pos.into_iter().next();

        // Allocate the empty backing dict first so tallying can use the
        // `HeapRead<Dict>` get/set API (which needs a heap-resident dict). Guard
        // its owned ref so it is freed if tallying fails partway.
        let dict_id = vm.heap.allocate(HeapData::Dict(Dict::new()))?;
        let mut dict_guard = HeapGuard::new(Value::Ref(dict_id), vm);
        {
            let (_backing, vm) = dict_guard.as_parts_mut();
            if let Some(source) = source
                && let Err(e) = counter_add_from_source(dict_id, source, 1, vm)
            {
                kwargs.drop_with_heap(vm);
                return Err(e);
            }
            counter_add_from_kwargs(dict_id, kwargs, 1, vm)?;
        }
        // Transfer the backing dict's owned ref to the wrapper: `forget` the
        // guard's `Value::Ref` so its count is not decremented here — the
        // wrapper now owns it and releases it via `py_dec_ref_ids`.
        mem::forget(dict_guard.into_inner());
        Self::wrap(vm, dict_id, DictSubclassKind::Counter)
    }

    /// Allocates the backing `dict` from an already-built `Dict`, then wraps it.
    fn finish(vm: &mut VM<'_, impl ResourceTracker>, dict: Dict, kind: DictSubclassKind) -> RunResult<Value> {
        let dict_id = vm.heap.allocate(HeapData::Dict(dict))?;
        Self::wrap(vm, dict_id, kind)
    }

    /// Wraps an already-allocated backing dict (whose owned ref is transferred
    /// into the wrapper) in a `DictSubclass` heap entry.
    ///
    /// A failure here is a terminal `ResourceError` (allocator over budget), so
    /// the leaked backing-dict/factory refs on that path are acceptable per the
    /// project's resource-exhaustion contract.
    fn wrap(vm: &mut VM<'_, impl ResourceTracker>, dict_id: HeapId, kind: DictSubclassKind) -> RunResult<Value> {
        let id = vm.heap.allocate(HeapData::DictSubclass(Self { dict_id, kind }))?;
        Ok(Value::Ref(id))
    }
}

impl<'h> HeapRead<'h, DictSubclass> {
    /// Returns the backing dict's HeapId.
    fn inner_id(&self, vm: &VM<'h, impl ResourceTracker>) -> HeapId {
        self.get(vm.heap).dict_id
    }

    /// Reads the backing dict as a `HeapRead<Dict>`.
    fn inner<'a>(&self, vm: &'a mut VM<'h, impl ResourceTracker>) -> HeapRead<'h, Dict> {
        let dict_id = self.inner_id(vm);
        match vm.heap.read(dict_id) {
            HeapReadOutput::Dict(dict) => dict,
            _ => unreachable!("DictSubclass.dict_id must reference a Dict"),
        }
    }

    /// Whether this is a `Counter` (vs `defaultdict`).
    fn is_counter(&self, vm: &VM<'h, impl ResourceTracker>) -> bool {
        matches!(self.get(vm.heap).kind, DictSubclassKind::Counter)
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, DictSubclass> {
    fn py_type(&self, vm: &VM<'h, impl ResourceTracker>) -> Type {
        match self.get(vm.heap).kind {
            DictSubclassKind::DefaultDict { .. } => Type::DefaultDict,
            DictSubclassKind::Counter => Type::Counter,
        }
    }

    fn py_len(&self, vm: &VM<'h, impl ResourceTracker>) -> Option<usize> {
        let dict_id = self.get(vm.heap).dict_id;
        match vm.heap.read(dict_id) {
            HeapReadOutput::Dict(dict) => Some(dict.get(vm.heap).len()),
            _ => None,
        }
    }

    fn py_bool(&self, vm: &mut VM<'h, impl ResourceTracker>) -> bool {
        let dict = self.inner(vm);
        !dict.get(vm.heap).is_empty()
    }

    fn py_getitem(&self, key: &Value, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Value> {
        let dict = self.inner(vm);
        if let Some(value) = dict.dict_get(key, vm)? {
            return Ok(value);
        }
        // Miss: Counter returns 0; defaultdict calls its factory (builtins only).
        match &self.get(vm.heap).kind {
            DictSubclassKind::Counter => Ok(Value::Int(0)),
            DictSubclassKind::DefaultDict { factory } => {
                if matches!(factory, Value::None) {
                    return Err(ExcType::key_error(key, vm));
                }
                let factory = factory.clone_with_heap(vm.heap);
                defer_drop!(factory, vm);
                let default = call_default_factory(factory, vm)?;
                // Insert key -> default into the backing dict and return default.
                let mut dict = self.inner(vm);
                let key_clone = key.clone_with_heap(vm.heap);
                let default_clone = default.clone_with_heap(vm.heap);
                if let Some(old) = dict.set(key_clone, default_clone, vm)? {
                    old.drop_with_heap(vm);
                }
                Ok(default)
            }
        }
    }

    fn py_setitem(&mut self, key: Value, value: Value, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<()> {
        let mut dict = self.inner(vm);
        if let Some(old_value) = dict.set(key, value, vm)? {
            old_value.drop_with_heap(vm);
        }
        Ok(())
    }

    fn py_eq_impl(&self, other: &Value, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Option<bool>> {
        // A dict subclass compares equal to a plain dict or another subclass with
        // the same items (kind/factory are ignored), matching CPython.
        let other_dict_id = match other {
            Value::Ref(oid) => match vm.heap.get(*oid) {
                HeapData::Dict(_) => *oid,
                HeapData::DictSubclass(sub) => sub.dict_id(),
                _ => return Ok(None),
            },
            _ => return Ok(None),
        };
        let self_dict_id = self.get(vm.heap).dict_id;
        let HeapReadOutput::Dict(a) = vm.heap.read(self_dict_id) else {
            unreachable!("backing dict")
        };
        let HeapReadOutput::Dict(b) = vm.heap.read(other_dict_id) else {
            unreachable!("backing dict")
        };
        Ok(Some(a.eq_dict(&b, vm)?))
    }

    fn py_repr_fmt(
        &self,
        f: &mut impl Write,
        vm: &mut VM<'h, impl ResourceTracker>,
        heap_ids: &mut LazyHeapSet,
    ) -> RunResult<()> {
        match &self.get(vm.heap).kind {
            DictSubclassKind::DefaultDict { factory } => {
                let factory = factory.clone_with_heap(vm.heap);
                defer_drop!(factory, vm);
                f.write_str("defaultdict(")?;
                factory.py_repr_fmt(f, vm, heap_ids)?;
                f.write_str(", ")?;
                let dict = self.inner(vm);
                dict.py_repr_fmt(f, vm, heap_ids)?;
                f.write_char(')')?;
                Ok(())
            }
            DictSubclassKind::Counter => counter_repr_fmt(self, f, vm, heap_ids),
        }
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Option<CallResult>> {
        // `defaultdict.default_factory` is the only attribute; everything else is
        // a method (via py_call_attr) or absent.
        if attr.static_string() == Some(StaticStrings::DefaultFactory)
            && let DictSubclassKind::DefaultDict { factory } = &self.get(vm.heap).kind
        {
            let value = factory.clone_with_heap(vm.heap);
            Ok(Some(CallResult::Value(value)))
        } else {
            Ok(None)
        }
    }

    fn py_call_attr(
        &mut self,
        _self_id: HeapId,
        vm: &mut VM<'h, impl ResourceTracker>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        let Some(method) = attr.static_string() else {
            args.drop_with_heap(vm);
            return Err(ExcType::attribute_error(self.py_type(vm), attr.as_str(vm.interns)));
        };
        let counter = self.is_counter(vm);
        match method {
            // Same-kind-preserving copy.
            StaticStrings::Copy => {
                args.check_zero_args("copy", vm.heap)?;
                self.copy_subclass(vm).map(CallResult::Value)
            }
            StaticStrings::MostCommon if counter => counter_most_common(self, args, vm).map(CallResult::Value),
            StaticStrings::Elements if counter => counter_elements(self, args, vm).map(CallResult::Value),
            StaticStrings::Subtract if counter => counter_subtract(self, args, vm).map(CallResult::Value),
            StaticStrings::Update if counter => counter_update(self, args, vm).map(CallResult::Value),
            // Shared dict methods delegate to the backing dict. Views reference
            // the backing dict id, so they iterate correctly.
            StaticStrings::Get
            | StaticStrings::Keys
            | StaticStrings::Values
            | StaticStrings::Items
            | StaticStrings::Pop
            | StaticStrings::Popitem
            | StaticStrings::Clear
            | StaticStrings::Setdefault
            | StaticStrings::Update => {
                let dict_id = self.inner_id(vm);
                let HeapReadOutput::Dict(mut dict) = vm.heap.read(dict_id) else {
                    unreachable!("backing dict")
                };
                dict.py_call_attr(dict_id, vm, attr, args)
            }
            _ => {
                args.drop_with_heap(vm);
                Err(ExcType::attribute_error(self.py_type(vm), attr.as_str(vm.interns)))
            }
        }
    }
}

impl<'h> HeapRead<'h, DictSubclass> {
    /// Returns a shallow copy preserving the subclass kind (and factory).
    fn copy_subclass(&self, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Value> {
        // Clone the backing dict's entries into a fresh dict.
        let dict = self.inner(vm);
        let len = dict.get(vm.heap).len();
        let mut pairs = Vec::with_capacity(len);
        for i in 0..len {
            let key = dict
                .get(vm.heap)
                .key_at(i)
                .expect("index in range")
                .clone_with_heap(vm.heap);
            let value = dict
                .get(vm.heap)
                .value_at(i)
                .expect("index in range")
                .clone_with_heap(vm.heap);
            pairs.push((key, value));
        }
        let new_dict = Dict::from_pairs(pairs, vm)?;
        let kind = match &self.get(vm.heap).kind {
            DictSubclassKind::DefaultDict { factory } => DictSubclassKind::DefaultDict {
                factory: factory.clone_with_heap(vm.heap),
            },
            DictSubclassKind::Counter => DictSubclassKind::Counter,
        };
        DictSubclass::finish(vm, new_dict, kind)
    }
}

impl HeapItem for DictSubclass {
    fn py_estimate_size(&self) -> usize {
        mem::size_of::<Self>() + VALUE_SIZE
    }

    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        stack.push(self.dict_id);
        if let DictSubclassKind::DefaultDict { factory } = &mut self.kind {
            factory.py_dec_ref_ids(stack);
        }
    }
}

/// Calls a `defaultdict` factory to produce a default value.
///
/// Only builtin callables (e.g. `int`, `list`, `set`, `dict`) are supported —
/// they can be invoked synchronously. User-defined function factories cannot be
/// called from `__getitem__` (which cannot push a VM frame), so they raise a
/// `TypeError`; this divergence is documented in `limitations/collections.md`.
fn call_default_factory(factory: &Value, vm: &mut VM<'_, impl ResourceTracker>) -> RunResult<Value> {
    match factory {
        Value::Builtin(builtin) => match builtin.call(vm, ArgValues::Empty)? {
            CallResult::Value(v) => Ok(v),
            _ => Err(SimpleException::new_msg(
                ExcType::TypeError,
                "defaultdict default_factory returned a non-value result",
            )
            .into()),
        },
        _ => Err(SimpleException::new_msg(
            ExcType::TypeError,
            "defaultdict with a non-builtin default_factory is not supported in Monty",
        )
        .into()),
    }
}

/// Returns whether `value` is callable (usable as a `defaultdict` factory).
fn is_callable(value: &Value, vm: &VM<'_, impl ResourceTracker>) -> bool {
    match value {
        Value::DefFunction(_) | Value::Builtin(_) | Value::ExtFunction(_) | Value::ModuleFunction(_) => true,
        Value::Ref(id) => matches!(
            vm.heap.get(*id),
            HeapData::Closure(_) | HeapData::FunctionDefaults(_) | HeapData::ExtFunction(_) | HeapData::Class(_)
        ),
        _ => false,
    }
}

/// Writes `Counter`'s repr: `Counter()` when empty, otherwise
/// `Counter({k: v, ...})` with entries ordered by count descending (ties keep
/// insertion order), matching CPython.
fn counter_repr_fmt<'h>(
    counter: &HeapRead<'h, DictSubclass>,
    f: &mut impl Write,
    vm: &mut VM<'h, impl ResourceTracker>,
    heap_ids: &mut LazyHeapSet,
) -> RunResult<()> {
    let order = counter_indices_by_count(counter, vm);
    if order.is_empty() {
        return Ok(f.write_str("Counter()")?);
    }
    let Ok(mut guard) = vm.recursion_guard() else {
        return Ok(f.write_str("Counter({...})")?);
    };
    let vm = &mut *guard;
    f.write_str("Counter({")?;
    for (i, &idx) in order.iter().enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        let dict = counter.inner(vm);
        let key = dict
            .get(vm.heap)
            .key_at(idx)
            .expect("index in range")
            .clone_with_heap(vm.heap);
        defer_drop!(key, vm);
        key.py_repr_fmt(f, vm, heap_ids)?;
        f.write_str(": ")?;
        let dict = counter.inner(vm);
        let value = dict
            .get(vm.heap)
            .value_at(idx)
            .expect("index in range")
            .clone_with_heap(vm.heap);
        defer_drop!(value, vm);
        value.py_repr_fmt(f, vm, heap_ids)?;
    }
    f.write_str("})")?;
    Ok(())
}

/// Implements `Counter.most_common([n])`, returning a list of `(elem, count)`
/// tuples ordered by count descending (ties keep insertion order). `n` limits
/// the result to the top-n; omitting it returns all entries.
fn counter_most_common<'h>(
    counter: &HeapRead<'h, DictSubclass>,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    let n = match args.get_zero_one_arg("most_common", vm.heap)? {
        Some(v) if matches!(v, Value::None) => {
            v.drop_with_heap(vm.heap);
            None
        }
        Some(v) => {
            let result = v.as_int(vm);
            v.drop_with_heap(vm.heap);
            Some(result?)
        }
        None => None,
    };
    let order = counter_indices_by_count(counter, vm);
    let limit = match n {
        Some(n) if n < 0 => 0,
        Some(n) => usize::try_from(n).unwrap_or(usize::MAX).min(order.len()),
        None => order.len(),
    };
    let mut items = Vec::with_capacity(limit);
    for &idx in order.iter().take(limit) {
        let dict = counter.inner(vm);
        let key = dict
            .get(vm.heap)
            .key_at(idx)
            .expect("index in range")
            .clone_with_heap(vm.heap);
        let value = dict
            .get(vm.heap)
            .value_at(idx)
            .expect("index in range")
            .clone_with_heap(vm.heap);
        let pair = allocate_tuple(smallvec::smallvec![key, value], vm.heap)?;
        items.push(pair);
    }
    let id = vm.heap.allocate(HeapData::List(List::new(items)))?;
    Ok(Value::Ref(id))
}

/// Implements `Counter.elements()`, returning a list that repeats each element
/// by its count (elements with count <= 0 are skipped), in insertion order.
///
/// CPython returns a lazy `itertools.chain` iterator; Monty returns a `list`
/// (documented in `limitations/collections.md`).
fn counter_elements<'h>(
    counter: &HeapRead<'h, DictSubclass>,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    args.check_zero_args("elements", vm.heap)?;
    let dict = counter.inner(vm);
    let len = dict.get(vm.heap).len();
    let mut items: Vec<Value> = Vec::new();
    for i in 0..len {
        let count = dict
            .get(vm.heap)
            .value_at(i)
            .expect("index in range")
            .as_int(vm)
            .unwrap_or(0);
        for _ in 0..count.max(0) {
            let key = dict
                .get(vm.heap)
                .key_at(i)
                .expect("index in range")
                .clone_with_heap(vm.heap);
            vm.heap.track_growth(VALUE_SIZE)?;
            items.push(key);
        }
    }
    let id = vm.heap.allocate(HeapData::List(List::new(items)))?;
    Ok(Value::Ref(id))
}

/// Implements `Counter.update(other)`, adding counts from another mapping,
/// Counter, or iterable (and keyword arguments).
fn counter_update<'h>(
    counter: &mut HeapRead<'h, DictSubclass>,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    counter_update_signed(counter, args, 1, "Counter.update", vm)
}

/// Implements `Counter.subtract(other)`, subtracting counts (results may be
/// zero or negative).
fn counter_subtract<'h>(
    counter: &mut HeapRead<'h, DictSubclass>,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    counter_update_signed(counter, args, -1, "Counter.subtract", vm)
}

/// Shared body for `Counter.update`/`Counter.subtract`: applies `sign * count`
/// from the source and keyword arguments to the backing dict.
fn counter_update_signed<'h>(
    counter: &mut HeapRead<'h, DictSubclass>,
    args: ArgValues,
    sign: i64,
    name: &str,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    let dict_id = counter.inner_id(vm);
    let (pos, kwargs) = args.into_parts();
    let pos: Vec<Value> = pos.collect();
    if pos.len() > 1 {
        let actual = pos.len() + 1;
        pos.drop_with_heap(vm);
        kwargs.drop_with_heap(vm);
        return Err(ExcType::type_error_too_many_positional_range(name, 1, 2, actual, 0));
    }
    if let Some(source) = pos.into_iter().next()
        && let Err(e) = counter_add_from_source(dict_id, source, sign, vm)
    {
        kwargs.drop_with_heap(vm);
        return Err(e);
    }
    counter_add_from_kwargs(dict_id, kwargs, sign, vm)?;
    Ok(Value::None)
}

/// Adds `sign * count` for each element of `source` into the backing dict.
///
/// A mapping (`dict`/`Counter`) contributes each value as the count; any other
/// iterable contributes 1 per element.
fn counter_add_from_source(
    dict_id: HeapId,
    source: Value,
    sign: i64,
    vm: &mut VM<'_, impl ResourceTracker>,
) -> RunResult<()> {
    let mut source_guard = HeapGuard::new(source, vm);
    // Detect a mapping: a plain dict or another dict subclass.
    let mapping_dict_id = {
        let (source_ref, vm) = source_guard.as_parts();
        match source_ref {
            Value::Ref(id) => match vm.heap.get(*id) {
                HeapData::Dict(_) => Some(*id),
                HeapData::DictSubclass(sub) => Some(sub.dict_id()),
                _ => None,
            },
            _ => None,
        }
    };

    if let Some(map_id) = mapping_dict_id {
        let vm = source_guard.heap();
        // Snapshot (key, count) pairs from the mapping first, so updating a
        // Counter from itself (`c.update(c)`) sees a stable source.
        let HeapReadOutput::Dict(map) = vm.heap.read(map_id) else {
            unreachable!("mapping dict")
        };
        let len = map.get(vm.heap).len();
        let mut pairs = Vec::with_capacity(len);
        for i in 0..len {
            let key = map
                .get(vm.heap)
                .key_at(i)
                .expect("index in range")
                .clone_with_heap(vm.heap);
            let count = map
                .get(vm.heap)
                .value_at(i)
                .expect("index in range")
                .as_int(vm)
                .unwrap_or(0);
            pairs.push((key, count));
        }
        drop(map);
        for (key, count) in pairs {
            counter_add_one(dict_id, key, sign * count, vm)?;
        }
        Ok(())
    } else {
        // Iterable of hashable elements: `sign` per occurrence.
        let source = source_guard.into_inner();
        let items: Vec<Value> = MontyIter::new(source, vm)?.collect(vm)?;
        for item in items {
            counter_add_one(dict_id, item, sign, vm)?;
        }
        Ok(())
    }
}

/// Adds `sign * count` from keyword arguments into the backing dict.
fn counter_add_from_kwargs(
    dict_id: HeapId,
    kwargs: KwargsValues,
    sign: i64,
    vm: &mut VM<'_, impl ResourceTracker>,
) -> RunResult<()> {
    let kwargs_iter = kwargs.into_iter();
    defer_drop_mut!(kwargs_iter, vm);
    for (key, value) in kwargs_iter {
        let count = value.as_int(vm).unwrap_or(0);
        value.drop_with_heap(vm);
        counter_add_one(dict_id, key, sign * count, vm)?;
    }
    Ok(())
}

/// Adds `delta` to the count for `key` in the backing dict (consuming `key`'s
/// ownership). The stored value stays an `int`; zero/negative results are kept
/// (only the arithmetic operators drop non-positive counts).
fn counter_add_one(dict_id: HeapId, key: Value, delta: i64, vm: &mut VM<'_, impl ResourceTracker>) -> RunResult<()> {
    let HeapReadOutput::Dict(mut dict) = vm.heap.read(dict_id) else {
        unreachable!("backing dict")
    };
    let current = match dict.dict_get(&key, vm)? {
        Some(v) => {
            let n = v.as_int(vm).unwrap_or(0);
            v.drop_with_heap(vm);
            n
        }
        None => 0,
    };
    if let Some(old) = dict.set(key, Value::Int(current + delta), vm)? {
        old.drop_with_heap(vm);
    }
    Ok(())
}

/// The four `Counter` arithmetic operators. All produce a new `Counter` and
/// discard non-positive results.
#[derive(Debug, Clone, Copy)]
pub(crate) enum CounterOp {
    /// `+` — add counts.
    Add,
    /// `-` — subtract counts.
    Sub,
    /// `&` — minimum of counts (multiset intersection).
    And,
    /// `|` — maximum of counts (multiset union).
    Or,
}

/// Computes a `Counter` binary operation (`+`, `-`, `&`, `|`).
///
/// Returns `Ok(None)` if either operand is not a `Counter` (so the caller falls
/// back to the standard `TypeError`). Follows CPython's algorithm: results with
/// a non-positive count are dropped, and iteration order is `self`'s keys first,
/// then `other`'s new keys.
pub(crate) fn counter_binop<'h>(
    a: &HeapRead<'h, DictSubclass>,
    b: &HeapRead<'h, DictSubclass>,
    op: CounterOp,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Option<Value>> {
    if !a.is_counter(vm) || !b.is_counter(vm) {
        return Ok(None);
    }
    let a_id = a.inner_id(vm);
    let b_id = b.inner_id(vm);
    let a_entries = snapshot_counts(a_id, vm);

    let result_id = vm.heap.allocate(HeapData::Dict(Dict::new()))?;
    let mut result_guard = HeapGuard::new(Value::Ref(result_id), vm);
    {
        let (_backing, vm) = result_guard.as_parts_mut();

        // First pass: every key of `self`.
        for (key, self_count) in a_entries {
            let other_count = counter_lookup(b_id, &key, vm)?;
            let newcount = match op {
                CounterOp::Add => self_count + other_count,
                CounterOp::Sub => self_count - other_count,
                CounterOp::And => self_count.min(other_count),
                CounterOp::Or => self_count.max(other_count),
            };
            if newcount > 0 {
                counter_set(result_id, key, newcount, vm)?;
            } else {
                key.drop_with_heap(vm);
            }
        }

        // Second pass: keys only in `other`. `&` is intersection, so it skips this.
        if !matches!(op, CounterOp::And) {
            let b_entries = snapshot_counts(b_id, vm);
            for (key, other_count) in b_entries {
                let in_self = counter_contains(a_id, &key, vm)?;
                let newcount = match op {
                    CounterOp::Sub => -other_count, // negatives become positive magnitudes
                    _ => other_count,
                };
                if !in_self && newcount > 0 {
                    counter_set(result_id, key, newcount, vm)?;
                } else {
                    key.drop_with_heap(vm);
                }
            }
        }
    }

    // Transfer the result dict's owned ref to the new Counter wrapper: `forget`
    // the guard's `Value::Ref` so its count is not decremented here.
    mem::forget(result_guard.into_inner());
    DictSubclass::wrap(vm, result_id, DictSubclassKind::Counter).map(Some)
}

/// Snapshots a backing dict's `(key_clone, count)` entries.
fn snapshot_counts(dict_id: HeapId, vm: &mut VM<'_, impl ResourceTracker>) -> Vec<(Value, i64)> {
    let HeapReadOutput::Dict(dict) = vm.heap.read(dict_id) else {
        unreachable!("backing dict")
    };
    let len = dict.get(vm.heap).len();
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let key = dict
            .get(vm.heap)
            .key_at(i)
            .expect("index in range")
            .clone_with_heap(vm.heap);
        let count = dict
            .get(vm.heap)
            .value_at(i)
            .expect("index in range")
            .as_int(vm)
            .unwrap_or(0);
        out.push((key, count));
    }
    out
}

/// Looks up `key`'s count in a backing dict, returning 0 if absent.
fn counter_lookup(dict_id: HeapId, key: &Value, vm: &mut VM<'_, impl ResourceTracker>) -> RunResult<i64> {
    let HeapReadOutput::Dict(dict) = vm.heap.read(dict_id) else {
        unreachable!("backing dict")
    };
    match dict.dict_get(key, vm)? {
        Some(v) => {
            let n = v.as_int(vm).unwrap_or(0);
            v.drop_with_heap(vm);
            Ok(n)
        }
        None => Ok(0),
    }
}

/// Returns whether `key` is present in a backing dict.
fn counter_contains(dict_id: HeapId, key: &Value, vm: &mut VM<'_, impl ResourceTracker>) -> RunResult<bool> {
    let HeapReadOutput::Dict(dict) = vm.heap.read(dict_id) else {
        unreachable!("backing dict")
    };
    dict.contains_key(key, vm)
}

/// Sets `key -> count` (an `int`) in a backing dict, consuming `key`.
fn counter_set(dict_id: HeapId, key: Value, count: i64, vm: &mut VM<'_, impl ResourceTracker>) -> RunResult<()> {
    let HeapReadOutput::Dict(mut dict) = vm.heap.read(dict_id) else {
        unreachable!("backing dict")
    };
    if let Some(old) = dict.set(key, Value::Int(count), vm)? {
        old.drop_with_heap(vm);
    }
    Ok(())
}

/// Returns the backing-dict entry indices of a Counter ordered by count
/// descending, with ties in insertion order. Returns indices (not clones) so
/// callers only clone the entries they actually use. Used by repr and
/// `most_common`.
fn counter_indices_by_count<'h>(
    counter: &HeapRead<'h, DictSubclass>,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> Vec<usize> {
    let dict = counter.inner(vm);
    let len = dict.get(vm.heap).len();
    let mut idx_count: Vec<(usize, i64)> = Vec::with_capacity(len);
    for i in 0..len {
        let count = dict
            .get(vm.heap)
            .value_at(i)
            .expect("index in range")
            .as_int(vm)
            .unwrap_or(0);
        idx_count.push((i, count));
    }
    // Stable sort by count descending; the original index as a tiebreaker
    // preserves insertion order for equal counts.
    idx_count.sort_by_key(|(idx, count)| (Reverse(*count), *idx));
    idx_count.into_iter().map(|(i, _)| i).collect()
}
