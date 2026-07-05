use std::{cmp::Ordering, collections::VecDeque, fmt::Write, mem};

use super::{CmpOrder, MontyIter, PyTrait};
use crate::{
    args::{ArgValues, FromArgs},
    bytecode::{CallResult, ContainsVM, DropWithVM, RecursionToken, VM},
    defer_drop, defer_drop_mut, defer_drop_vm_mut,
    exception_private::{ExcType, RunError, RunResult, SimpleException},
    heap::{DropWithHeap, HeapData, HeapId, HeapItem, HeapRead, HeapReadOutput},
    intern::StaticStrings,
    resource::ResourceTracker,
    types::{LazyHeapSet, Type, slice::normalize_sequence_index},
    value::{EitherStr, VALUE_SIZE, Value},
};

/// Python `collections.deque` type, a double-ended queue backed by a `VecDeque`.
///
/// Supports O(1) appends and pops from both ends. An optional `maxlen` bounds
/// the queue: once full, an append on one end discards an item from the other,
/// matching CPython's "bounded deque" semantics (e.g. a fixed-size ring buffer).
///
/// Unlike `list`, deques do **not** support slicing (`d[1:2]` raises
/// `TypeError`), only integer indexing. Equality compares element-wise against
/// another deque; a deque never compares equal to a `list`.
///
/// # Reference counting
/// Items transferred into the deque have already had their refcounts handled by
/// the caller (as with [`List`](super::List)); items discarded due to `maxlen`
/// overflow are dropped via `drop_with_heap`.
///
/// # GC optimization
/// `contains_refs` tracks whether any item is a `Value::Ref`, letting
/// `py_dec_ref_ids` skip iteration for deques of primitives.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Deque {
    items: VecDeque<Value>,
    /// Maximum length; `None` means unbounded.
    maxlen: Option<usize>,
    /// True if any item is a `Value::Ref` (GC fast-path flag).
    contains_refs: bool,
}

impl Deque {
    /// Creates a deque from a `VecDeque` of values and an optional `maxlen`.
    ///
    /// Does NOT enforce `maxlen` on the passed items or manage refcounts — the
    /// caller must ensure `items.len() <= maxlen` and that refcounts are owned.
    #[must_use]
    pub fn new(items: VecDeque<Value>, maxlen: Option<usize>) -> Self {
        let contains_refs = items.iter().any(|v| matches!(v, Value::Ref(_)));
        Self {
            items,
            maxlen,
            contains_refs,
        }
    }

    /// Returns a reference to the underlying `VecDeque`.
    #[must_use]
    pub fn as_deque(&self) -> &VecDeque<Value> {
        &self.items
    }

    /// Returns whether the deque contains any heap references.
    #[inline]
    #[must_use]
    pub fn contains_refs(&self) -> bool {
        self.contains_refs
    }
}

impl Deque {
    /// Implements the `deque(iterable=(), maxlen=None)` constructor.
    pub fn init(vm: &mut VM<'_, impl ResourceTracker>, args: ArgValues) -> RunResult<Value> {
        let DequeInitArgs { iterable, maxlen } = DequeInitArgs::from_args(args, vm)?;
        defer_drop!(iterable, vm);
        defer_drop!(maxlen, vm);

        let maxlen = parse_maxlen(maxlen, vm)?;

        let mut items: VecDeque<Value> = VecDeque::new();
        if !matches!(*iterable, Value::None) {
            let collected: Vec<Value> = MontyIter::new(iterable.clone_with_heap(vm.heap), vm)?.collect(vm)?;
            vm.heap.track_growth(collected.len() * VALUE_SIZE)?;
            items.extend(collected);
            // Enforce maxlen by discarding from the front, matching CPython.
            if let Some(max) = maxlen {
                while items.len() > max {
                    if let Some(dropped) = items.pop_front() {
                        dropped.drop_with_heap(vm.heap);
                    }
                }
            }
        }

        let heap_id = vm.heap.allocate(HeapData::Deque(Self::new(items, maxlen)))?;
        Ok(Value::Ref(heap_id))
    }
}

/// Parses the `maxlen` constructor argument into `Option<usize>`.
///
/// `None` stays unbounded; an integer must be non-negative (CPython raises
/// `ValueError: maxlen must be non-negative` otherwise), and non-int types
/// raise `TypeError`.
fn parse_maxlen(maxlen: &Value, vm: &VM<'_, impl ResourceTracker>) -> RunResult<Option<usize>> {
    if matches!(maxlen, Value::None) {
        Ok(None)
    } else {
        let n = maxlen.as_int(vm)?;
        if n < 0 {
            Err(SimpleException::new_msg(ExcType::ValueError, "maxlen must be non-negative").into())
        } else {
            Ok(Some(usize::try_from(n).expect("non-negative i64 fits usize on 64-bit")))
        }
    }
}

/// Constructor arguments for `deque(iterable=(), maxlen=None)`.
///
/// Both are `Value` so the body can coerce `maxlen` and produce CPython-matching
/// errors (deque is C-implemented, so a bad `maxlen` type is reported by the
/// body, not during binding).
#[derive(FromArgs)]
#[from_args(name = "deque")]
struct DequeInitArgs {
    #[from_args(default = Value::None)]
    iterable: Value,
    #[from_args(static_string = "Maxlen", default = Value::None)]
    maxlen: Value,
}

impl<'h> HeapRead<'h, Deque> {
    /// Appends an item to the right end, enforcing `maxlen` by discarding from
    /// the left. Ownership of `item` transfers to the deque (refcount already
    /// handled by the caller).
    pub fn append(&mut self, vm: &mut VM<'h, impl ResourceTracker>, item: Value) -> RunResult<()> {
        if matches!(item, Value::Ref(_)) {
            self.get_mut(vm.heap).contains_refs = true;
        }
        let maxlen = self.get(vm.heap).maxlen;
        if maxlen == Some(0) {
            // A zero-maxlen deque silently discards everything.
            item.drop_with_heap(vm.heap);
            return Ok(());
        }
        vm.heap.track_growth(VALUE_SIZE)?;
        self.get_mut(vm.heap).items.push_back(item);
        if let Some(max) = maxlen
            && self.get(vm.heap).items.len() > max
            && let Some(dropped) = self.get_mut(vm.heap).items.pop_front()
        {
            dropped.drop_with_heap(vm.heap);
        }
        Ok(())
    }

    /// Appends an item to the left end, enforcing `maxlen` by discarding from
    /// the right. Ownership of `item` transfers to the deque.
    pub fn appendleft(&mut self, vm: &mut VM<'h, impl ResourceTracker>, item: Value) -> RunResult<()> {
        if matches!(item, Value::Ref(_)) {
            self.get_mut(vm.heap).contains_refs = true;
        }
        let maxlen = self.get(vm.heap).maxlen;
        if maxlen == Some(0) {
            item.drop_with_heap(vm.heap);
            return Ok(());
        }
        vm.heap.track_growth(VALUE_SIZE)?;
        self.get_mut(vm.heap).items.push_front(item);
        if let Some(max) = maxlen
            && self.get(vm.heap).items.len() > max
            && let Some(dropped) = self.get_mut(vm.heap).items.pop_back()
        {
            dropped.drop_with_heap(vm.heap);
        }
        Ok(())
    }

    /// Clones the item at `index` with proper refcount management.
    pub(crate) fn clone_item(&self, index: usize, vm: &mut VM<'h, impl ResourceTracker>) -> Value {
        self.get(vm.heap).items[index].clone_with_heap(vm.heap)
    }

    /// Normalizes a possibly-negative index against the current length,
    /// returning `Some(usize)` if in bounds. Uses `index + len` (not `-index`)
    /// to avoid overflow on `i64::MIN`.
    fn normalize_index(&self, index: i64, vm: &VM<'h, impl ResourceTracker>) -> Option<usize> {
        let len = i64::try_from(self.get(vm.heap).items.len()).ok()?;
        let normalized = if index < 0 { index + len } else { index };
        if normalized < 0 || normalized >= len {
            None
        } else {
            usize::try_from(normalized).ok()
        }
    }

    /// Returns a stack-borrowed lending iterator over the deque's items,
    /// holding a recursion-depth token for its lifetime. See [`DequeIter`].
    #[expect(clippy::iter_not_returning_iterator)]
    pub(crate) fn iter<R: ResourceTracker>(&self, vm: &mut VM<'h, R>) -> RunResult<DequeIter<'_, 'h>> {
        DequeIter::new(self, vm)
    }
}

/// Stack-borrowed lending iterator over a [`Deque`]'s items.
///
/// Same lending shape and recursion-token discipline as
/// [`ListIter`](super::list::ListIter): [`next`](Self::next) returns
/// `Option<&Value>`, owning the yielded item in `current` and dropping the
/// prior one on each call. MUST be wrapped in [`defer_drop_vm_mut!`] so the
/// token and in-flight item are released on every exit path. The length is
/// re-read each call so concurrent mutation cannot cause an out-of-bounds
/// panic.
pub(crate) struct DequeIter<'a, 'h> {
    deque: &'a HeapRead<'h, Deque>,
    index: usize,
    token: RecursionToken,
    current: Value,
}

impl<'a, 'h> DequeIter<'a, 'h> {
    fn new<R: ResourceTracker>(deque: &'a HeapRead<'h, Deque>, vm: &mut VM<'h, R>) -> RunResult<Self> {
        let token = vm.recursion_token()?;
        Ok(Self {
            deque,
            index: 0,
            token,
            current: Value::Undefined,
        })
    }

    pub(crate) fn next<'i, R: ResourceTracker>(&'i mut self, vm: &mut VM<'h, R>) -> RunResult<Option<&'i Value>> {
        mem::replace(&mut self.current, Value::Undefined).drop_with_heap(vm.heap);
        vm.heap.check_time()?;
        if self.index >= self.deque.get(vm.heap).items.len() {
            return Ok(None);
        }
        self.current = self.deque.get(vm.heap).items[self.index].clone_with_heap(vm.heap);
        self.index += 1;
        Ok(Some(&self.current))
    }

    pub(crate) fn next_with_index<'i, R: ResourceTracker>(
        &'i mut self,
        vm: &mut VM<'h, R>,
    ) -> RunResult<Option<(usize, &'i Value)>> {
        let position = self.index;
        Ok(self.next(vm)?.map(|item| (position, item)))
    }
}

impl<'h> DropWithVM<'h> for DequeIter<'_, 'h> {
    fn drop_with_vm(self, container: &mut impl ContainsVM<'h>) {
        self.current.drop_with_heap(container);
        self.token.drop_with_vm(container);
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, Deque> {
    fn py_type(&self, _vm: &VM<'h, impl ResourceTracker>) -> Type {
        Type::Deque
    }

    fn py_len(&self, vm: &VM<'h, impl ResourceTracker>) -> Option<usize> {
        Some(self.get(vm.heap).items.len())
    }

    fn py_bool(&self, vm: &mut VM<'h, impl ResourceTracker>) -> bool {
        !self.get(vm.heap).items.is_empty()
    }

    fn py_getitem(&self, key: &Value, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Value> {
        let index = deque_key_to_index(key, vm)?;
        match self.normalize_index(index, vm) {
            Some(idx) => Ok(self.get(vm.heap).items[idx].clone_with_heap(vm.heap)),
            None => Err(deque_index_error()),
        }
    }

    fn py_setitem(&mut self, key: Value, value: Value, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<()> {
        defer_drop!(key, vm);
        defer_drop_mut!(value, vm);
        let index = deque_key_to_index(key, vm)?;
        let Some(idx) = self.normalize_index(index, vm) else {
            return Err(deque_index_error());
        };
        if matches!(*value, Value::Ref(_)) {
            self.get_mut(vm.heap).contains_refs = true;
        }
        mem::swap(&mut self.get_mut(vm.heap).items[idx], value);
        Ok(())
    }

    fn py_eq_impl(&self, other: &Value, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Option<bool>> {
        let Some(HeapReadOutput::Deque(other)) = other.read_heap(vm) else {
            return Ok(None);
        };
        if self.get(vm.heap).items.len() != other.get(vm.heap).items.len() {
            return Ok(Some(false));
        }
        let iter = self.iter(vm)?;
        defer_drop_vm_mut!(iter, vm);
        while let Some((i, a)) = iter.next_with_index(vm)? {
            let b = other.clone_item(i, vm);
            defer_drop!(b, vm);
            if !a.py_eq(b, vm)? {
                return Ok(Some(false));
            }
        }
        Ok(Some(true))
    }

    fn py_cmp(&self, other: &Self, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<CmpOrder> {
        let a_len = self.get(vm.heap).items.len();
        let b_len = other.get(vm.heap).items.len();
        let min_len = a_len.min(b_len);
        let iter = self.iter(vm)?;
        defer_drop_vm_mut!(iter, vm);
        while let Some((i, av)) = iter.next_with_index(vm)? {
            if i >= min_len {
                break;
            }
            let bv = other.clone_item(i, vm);
            defer_drop!(bv, vm);
            match av.py_cmp(bv, vm)? {
                CmpOrder::Ordered(Ordering::Equal) => {}
                CmpOrder::Ordered(ord) => return Ok(CmpOrder::Ordered(ord)),
                CmpOrder::Unordered => return Ok(CmpOrder::Unordered),
                CmpOrder::Incomparable => {
                    if !av.py_eq(bv, vm)? {
                        return Ok(CmpOrder::Incomparable);
                    }
                }
            }
        }
        Ok(CmpOrder::Ordered(a_len.cmp(&b_len)))
    }

    fn py_repr_fmt(
        &self,
        f: &mut impl Write,
        vm: &mut VM<'h, impl ResourceTracker>,
        heap_ids: &mut LazyHeapSet,
    ) -> RunResult<()> {
        let Ok(mut guard) = vm.recursion_guard() else {
            return Ok(f.write_str("...")?);
        };
        let vm = &mut *guard;

        f.write_str("deque([")?;
        let len = self.get(vm.heap).items.len();
        for i in 0..len {
            if i > 0 {
                if vm.heap.check_time().is_err() {
                    f.write_str(", ...[timeout]")?;
                    break;
                }
                f.write_str(", ")?;
            }
            let item = self.clone_item(i, vm);
            defer_drop!(item, vm);
            item.py_repr_fmt(f, vm, heap_ids)?;
        }
        f.write_char(']')?;
        if let Some(max) = self.get(vm.heap).maxlen {
            write!(f, ", maxlen={max}")?;
        }
        f.write_char(')')?;
        Ok(())
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
            return Err(ExcType::attribute_error(Type::Deque, attr.as_str(vm.interns)));
        };
        call_deque_method(self, method, args, vm).map(CallResult::Value)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Option<CallResult>> {
        // `maxlen` is a read-only attribute; everything else is a method
        // (resolved via py_call_attr) or unknown.
        if attr.static_string() == Some(StaticStrings::Maxlen) {
            let value = match self.get(vm.heap).maxlen {
                Some(max) => Value::Int(i64::try_from(max).expect("maxlen fits i64")),
                None => Value::None,
            };
            Ok(Some(CallResult::Value(value)))
        } else {
            Ok(None)
        }
    }
}

impl HeapItem for Deque {
    fn py_estimate_size(&self) -> usize {
        mem::size_of::<Self>() + self.items.len() * VALUE_SIZE
    }

    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        if !self.contains_refs {
            return;
        }
        for obj in &mut self.items {
            if let Value::Ref(id) = obj {
                stack.push(*id);
                #[cfg(feature = "memory-model-checks")]
                obj.dec_ref_forget();
            }
        }
    }
}

/// Builds the `IndexError: deque index out of range` exception.
fn deque_index_error() -> RunError {
    SimpleException::new_msg(ExcType::IndexError, "deque index out of range").into()
}

/// Converts a subscript key into an `i64` index for a deque.
///
/// Unlike lists, deques accept only integers (no slices): every non-integer
/// key — including `slice` — raises CPython's
/// `TypeError: sequence index must be integer, not '{type}'`. A `LongInt`
/// that overflows `i64` can never be in range, so it reports the same
/// out-of-range `IndexError` as any other oversized index.
fn deque_key_to_index(key: &Value, vm: &VM<'_, impl ResourceTracker>) -> RunResult<i64> {
    match key {
        Value::Int(i) => Ok(*i),
        Value::Bool(b) => Ok(i64::from(*b)),
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::LongInt(li) => li.to_i64().ok_or_else(deque_index_error),
            _ => Err(deque_type_index_error(key, vm)),
        },
        _ => Err(deque_type_index_error(key, vm)),
    }
}

/// Builds the `TypeError: sequence index must be integer, not '{type}'` raised
/// for a non-integer deque subscript.
fn deque_type_index_error(key: &Value, vm: &VM<'_, impl ResourceTracker>) -> RunError {
    SimpleException::new_msg(
        ExcType::TypeError,
        format!("sequence index must be integer, not '{}'", key.py_type_name(vm)),
    )
    .into()
}

/// Dispatches a method call on a deque value.
fn call_deque_method<'h>(
    deque: &mut HeapRead<'h, Deque>,
    method: StaticStrings,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    match method {
        StaticStrings::Append => {
            let item = args.get_one_arg("append", vm.heap)?;
            deque.append(vm, item)?;
            Ok(Value::None)
        }
        StaticStrings::Appendleft => {
            let item = args.get_one_arg("appendleft", vm.heap)?;
            deque.appendleft(vm, item)?;
            Ok(Value::None)
        }
        StaticStrings::Pop => {
            args.check_zero_args("pop", vm.heap)?;
            match deque.get_mut(vm.heap).items.pop_back() {
                Some(item) => Ok(item),
                None => Err(SimpleException::new_msg(ExcType::IndexError, "pop from an empty deque").into()),
            }
        }
        StaticStrings::Popleft => {
            args.check_zero_args("popleft", vm.heap)?;
            match deque.get_mut(vm.heap).items.pop_front() {
                Some(item) => Ok(item),
                None => Err(SimpleException::new_msg(ExcType::IndexError, "pop from an empty deque").into()),
            }
        }
        StaticStrings::Extend => deque_extend(deque, args, vm, false),
        StaticStrings::Extendleft => deque_extend(deque, args, vm, true),
        StaticStrings::Clear => {
            args.check_zero_args("clear", vm.heap)?;
            let items = mem::take(&mut deque.get_mut(vm.heap).items);
            Vec::from(items).drop_with_heap(vm.heap);
            Ok(Value::None)
        }
        StaticStrings::Copy => {
            args.check_zero_args("copy", vm.heap)?;
            deque_copy(deque, vm)
        }
        StaticStrings::Count => deque_count(deque, args, vm),
        StaticStrings::Index => deque_index_method(deque, args, vm),
        StaticStrings::Insert => deque_insert(deque, args, vm),
        StaticStrings::Remove => deque_remove(deque, args, vm),
        StaticStrings::Reverse => {
            args.check_zero_args("reverse", vm.heap)?;
            deque.get_mut(vm.heap).items = mem::take(&mut deque.get_mut(vm.heap).items).into_iter().rev().collect();
            Ok(Value::None)
        }
        StaticStrings::Rotate => deque_rotate(deque, args, vm),
        _ => {
            args.drop_with_heap(vm.heap);
            Err(ExcType::attribute_error(Type::Deque, method.into()))
        }
    }
}

/// Implements `deque.extend(iterable)` / `deque.extendleft(iterable)`.
///
/// `extendleft` appends each item to the left, which reverses their order in
/// the deque (matching CPython).
fn deque_extend<'h>(
    deque: &mut HeapRead<'h, Deque>,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
    left: bool,
) -> RunResult<Value> {
    let iterable = args.get_one_arg(if left { "extendleft" } else { "extend" }, vm.heap)?;
    let items: Vec<Value> = MontyIter::new(iterable, vm)?.collect(vm)?;
    for item in items {
        if left {
            deque.appendleft(vm, item)?;
        } else {
            deque.append(vm, item)?;
        }
    }
    Ok(Value::None)
}

/// Implements `deque.copy()`, returning a new deque with the same `maxlen`.
fn deque_copy<'h>(deque: &HeapRead<'h, Deque>, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Value> {
    let len = deque.get(vm.heap).items.len();
    let mut items = VecDeque::with_capacity(len);
    for i in 0..len {
        items.push_back(deque.clone_item(i, vm));
    }
    let maxlen = deque.get(vm.heap).maxlen;
    let heap_id = vm.heap.allocate(HeapData::Deque(Deque::new(items, maxlen)))?;
    Ok(Value::Ref(heap_id))
}

/// Implements `deque.count(value)`.
fn deque_count<'h>(
    deque: &HeapRead<'h, Deque>,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    let value = args.get_one_arg("count", vm.heap)?;
    defer_drop!(value, vm);
    let mut count: i64 = 0;
    let iter = deque.iter(vm)?;
    defer_drop_vm_mut!(iter, vm);
    while let Some(item) = iter.next(vm)? {
        if value.py_eq(item, vm)? {
            count += 1;
        }
    }
    Ok(Value::Int(count))
}

/// Implements `deque.index(value[, start[, stop]])`, raising `ValueError` when
/// the value is not found in the `[start, stop)` window.
fn deque_index_method<'h>(
    deque: &HeapRead<'h, Deque>,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    let pos_args = args.into_pos_only("index", vm.heap)?;
    defer_drop!(pos_args, vm);

    let len = deque.get(vm.heap).items.len();
    let (value, start, end) = match pos_args.as_slice() {
        [] => return Err(ExcType::type_error_at_least("index", 1, 0)),
        [value] => (value, 0, len),
        [value, start_arg] => {
            let start = normalize_sequence_index(start_arg.as_int(vm)?, len);
            (value, start, len)
        }
        [value, start_arg, end_arg] => {
            let start = normalize_sequence_index(start_arg.as_int(vm)?, len);
            let end = normalize_sequence_index(end_arg.as_int(vm)?, len).max(start);
            (value, start, end)
        }
        other => return Err(ExcType::type_error_at_most("index", 3, other.len())),
    };

    let iter = deque.iter(vm)?;
    defer_drop_vm_mut!(iter, vm);
    while let Some((i, item)) = iter.next_with_index(vm)? {
        if i >= end {
            break;
        }
        if i >= start && value.py_eq(item, vm)? {
            return Ok(Value::Int(i64::try_from(i).expect("index fits i64")));
        }
    }
    Err(SimpleException::new_msg(
        ExcType::ValueError,
        format!("{} is not in deque", repr_value(value, vm)?),
    )
    .into())
}

/// Implements `deque.insert(index, value)`, honoring `maxlen`.
fn deque_insert<'h>(
    deque: &mut HeapRead<'h, Deque>,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    let (index_obj, item) = args.get_two_args("insert", vm.heap)?;
    defer_drop!(index_obj, vm);
    defer_drop_mut!(item, vm);

    // A full bounded deque rejects insert, matching CPython.
    let len = deque.get(vm.heap).items.len();
    if let Some(max) = deque.get(vm.heap).maxlen
        && len >= max
    {
        return Err(SimpleException::new_msg(ExcType::IndexError, "deque already at its maximum size").into());
    }

    let index_i64 = index_obj.as_int(vm)?;
    let len_i64 = i64::try_from(len).expect("deque length fits i64");
    // Clamp per CPython: negatives add len then clamp to 0, large values append.
    let idx = if index_i64 < 0 {
        usize::try_from(index_i64 + len_i64).unwrap_or(0)
    } else {
        usize::try_from(index_i64).unwrap_or(len).min(len)
    };
    if matches!(*item, Value::Ref(_)) {
        deque.get_mut(vm.heap).contains_refs = true;
    }
    vm.heap.track_growth(VALUE_SIZE)?;
    let value = mem::replace(&mut *item, Value::Undefined);
    deque.get_mut(vm.heap).items.insert(idx, value);
    Ok(Value::None)
}

/// Implements `deque.remove(value)`, removing the first match.
fn deque_remove<'h>(
    deque: &mut HeapRead<'h, Deque>,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    let value = args.get_one_arg("remove", vm.heap)?;
    defer_drop!(value, vm);

    let mut found = None;
    {
        let iter = deque.iter(vm)?;
        defer_drop_vm_mut!(iter, vm);
        while let Some((i, item)) = iter.next_with_index(vm)? {
            if value.py_eq(item, vm)? {
                found = Some(i);
                break;
            }
        }
    }
    match found {
        Some(idx) => {
            if let Some(removed) = deque.get_mut(vm.heap).items.remove(idx) {
                removed.drop_with_heap(vm.heap);
            }
            Ok(Value::None)
        }
        None => Err(SimpleException::new_msg(
            ExcType::ValueError,
            format!("{} is not in deque", repr_value(value, vm)?),
        )
        .into()),
    }
}

/// Implements `deque.rotate(n=1)`: rotate `n` steps to the right (negative
/// rotates left). Rotations are taken modulo the length so large `n` is cheap.
fn deque_rotate<'h>(
    deque: &mut HeapRead<'h, Deque>,
    args: ArgValues,
    vm: &mut VM<'h, impl ResourceTracker>,
) -> RunResult<Value> {
    let n = match args.get_zero_one_arg("rotate", vm.heap)? {
        Some(v) => {
            let result = v.as_int(vm);
            v.drop_with_heap(vm.heap);
            result?
        }
        None => 1,
    };
    let len = deque.get(vm.heap).items.len();
    if len > 0 {
        let len_i64 = i64::try_from(len).expect("deque length fits i64");
        let shift = n.rem_euclid(len_i64);
        let shift = usize::try_from(shift).expect("shift in [0, len)");
        deque.get_mut(vm.heap).items.rotate_right(shift);
    }
    Ok(Value::None)
}

/// Renders `value`'s `repr()` as an owned `String` for error messages.
fn repr_value(value: &Value, vm: &mut VM<'_, impl ResourceTracker>) -> RunResult<String> {
    let repr = value.py_repr(vm)?;
    defer_drop!(repr, vm);
    let owned = repr.to_str(vm).map(str::to_owned).unwrap_or_default();
    Ok(owned)
}
