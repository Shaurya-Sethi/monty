use std::{fmt::Write, mem};

use ruff_python_stdlib::keyword::is_keyword;

use super::{Dict, LazyHeapSet, MontyIter, NamedTuple, PyTrait};
use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::{ExcType, RunResult, SimpleException},
    heap::{DropWithHeap, HeapData, HeapId, HeapItem, HeapRead, HeapReadOutput},
    intern::StaticStrings,
    resource::ResourceTracker,
    types::{
        Type, allocate_tuple,
        str::{allocate_string, str_isidentifier},
    },
    value::{EitherStr, Value},
};

/// A class object produced by `collections.namedtuple(...)`.
///
/// Calling it constructs a [`NamedTuple`] instance bound to `field_names` (with
/// trailing `defaults` applied). The class carries the class-level surface
/// (`__name__`, `_fields`, `_field_defaults`, `_make`); its own Python type is
/// `type` (like any class object).
///
/// Field names and the type name are stored as [`EitherStr::Heap`] strings
/// because the factory runs at *runtime*, after the interner is frozen.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct NamedTupleClass {
    /// The type name, e.g. `"Point"`.
    typename: EitherStr,
    /// Field names in definition order.
    field_names: Vec<EitherStr>,
    /// Default values applied to the *last* `defaults.len()` fields.
    defaults: Vec<Value>,
}

impl NamedTupleClass {
    /// Returns the type name.
    pub fn typename<'a>(&'a self, vm: &'a VM<'_, impl ResourceTracker>) -> &'a str {
        self.typename.as_str(vm.interns)
    }

    /// Returns the default values (for GC traversal).
    pub fn defaults(&self) -> &[Value] {
        &self.defaults
    }
}

/// Implements the `namedtuple(typename, field_names, *, rename=False,
/// defaults=None, module=None)` factory, returning a new [`NamedTupleClass`].
///
/// Validates the type name and field names (identifiers, non-keywords, no
/// leading underscore, no duplicates) exactly as CPython does, honoring
/// `rename` to auto-fix invalid field names to `_0`, `_1`, ….
pub fn make_namedtuple(vm: &mut VM<'_, impl ResourceTracker>, args: ArgValues) -> RunResult<Value> {
    let NamedtupleArgs {
        typename,
        field_names,
        rename,
        defaults,
    } = parse_namedtuple_args(args, vm)?;

    // Extract the field-name strings, then release the `field_names` argument
    // (it may be a heap list). `defaults` is dropped on every error path below.
    let names_result = split_field_names(&field_names, vm);
    field_names.drop_with_heap(vm);
    let mut names = match names_result {
        Ok(names) => names,
        Err(e) => {
            defaults.drop_with_heap(vm);
            return Err(e);
        }
    };

    // The typename and every field name must be a valid, non-keyword identifier.
    if let Err(e) = validate_name(&typename) {
        defaults.drop_with_heap(vm);
        return Err(e);
    }
    if rename {
        rename_invalid_fields(&mut names);
    }
    if let Err(e) = validate_field_names(&names, rename) {
        defaults.drop_with_heap(vm);
        return Err(e);
    }

    // `defaults` longer than the field list is a CPython error.
    if defaults.len() > names.len() {
        defaults.drop_with_heap(vm);
        return Err(SimpleException::new_msg(ExcType::TypeError, "Got more default values than field names").into());
    }

    let field_names: Vec<EitherStr> = names.into_iter().map(EitherStr::Heap).collect();
    let class = NamedTupleClass {
        typename: EitherStr::Heap(typename),
        field_names,
        defaults,
    };
    let id = vm.heap.allocate(HeapData::NamedTupleClass(class))?;
    Ok(Value::Ref(id))
}

/// Parsed and validated arguments for [`make_namedtuple`].
struct NamedtupleArgs {
    typename: String,
    field_names: Value,
    rename: bool,
    defaults: Vec<Value>,
}

/// Extracts the `namedtuple(...)` arguments, coercing `typename`/`rename` and
/// materializing `defaults` into an owned `Vec`.
fn parse_namedtuple_args(args: ArgValues, vm: &mut VM<'_, impl ResourceTracker>) -> RunResult<NamedtupleArgs> {
    let (pos, kwargs) = args.into_parts();
    let mut pos: Vec<Value> = pos.collect();

    let mut rename = false;
    let mut defaults_val = Value::None;
    // Bind keyword arguments (rename / defaults / module). `module` is accepted
    // and ignored — Monty has no real module objects for namedtuples.
    for (key, value) in kwargs {
        let key_name = key.to_str(vm).map(str::to_owned).unwrap_or_default();
        match key_name.as_str() {
            "rename" => rename = value.py_bool(vm),
            "defaults" => defaults_val = value.clone_with_heap(vm.heap),
            "module" => {}
            _ => {
                value.drop_with_heap(vm);
                pos.drop_with_heap(vm);
                defaults_val.drop_with_heap(vm);
                return Err(ExcType::type_error_unexpected_keyword("namedtuple", &key_name));
            }
        }
        value.drop_with_heap(vm);
    }

    if pos.len() < 2 || pos.len() > 2 {
        // namedtuple(typename, field_names, ...) requires exactly these two
        // positionals in Monty (rename/defaults/module are keyword-only here).
        let n = pos.len();
        pos.drop_with_heap(vm);
        defaults_val.drop_with_heap(vm);
        return Err(ExcType::type_error_too_many_positional_range("namedtuple", 2, 2, n, 0));
    }
    let field_names = pos.pop().expect("len == 2");
    let typename_val = pos.pop().expect("len == 2");

    let Ok(typename) = typename_val.to_str(vm).map(str::to_owned) else {
        let ty = typename_val.py_type_name(vm);
        typename_val.drop_with_heap(vm);
        field_names.drop_with_heap(vm);
        defaults_val.drop_with_heap(vm);
        return Err(ExcType::type_error(format!("expected str for typename, not {ty}")));
    };
    typename_val.drop_with_heap(vm);

    let defaults = if matches!(defaults_val, Value::None) {
        Vec::new()
    } else {
        MontyIter::new(defaults_val, vm)?.collect(vm)?
    };

    Ok(NamedtupleArgs {
        typename,
        field_names,
        rename,
        defaults,
    })
}

/// Splits the `field_names` argument into owned strings. A single string is
/// split on commas/whitespace (matching CPython); any other value is treated as
/// an iterable of strings.
fn split_field_names(field_names: &Value, vm: &mut VM<'_, impl ResourceTracker>) -> RunResult<Vec<String>> {
    if let Ok(s) = field_names.to_str(vm) {
        // CPython: field_names.replace(',', ' ').split()
        Ok(s.replace(',', " ").split_whitespace().map(str::to_owned).collect())
    } else {
        let items: Vec<Value> = MontyIter::new(field_names.clone_with_heap(vm.heap), vm)?.collect(vm)?;
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let result = item.to_str(vm).map(str::to_owned);
            item.drop_with_heap(vm);
            match result {
                Ok(s) => out.push(s),
                Err(_) => return Err(ExcType::type_error("field_names must be strings")),
            }
        }
        Ok(out)
    }
}

/// Rewrites invalid field names to positional `_N` placeholders (the effect of
/// `rename=True`): a name that is not an identifier, is a keyword, starts with
/// an underscore, or duplicates an earlier field becomes `_{index}`.
fn rename_invalid_fields(names: &mut [String]) {
    // `seen` tracks the *original* names (matching CPython), so a later
    // duplicate of a valid name is itself renamed.
    let mut seen: Vec<String> = Vec::with_capacity(names.len());
    for (i, name) in names.iter_mut().enumerate() {
        let original = name.clone();
        let invalid = !str_isidentifier(&original)
            || is_keyword(&original)
            || original.starts_with('_')
            || seen.contains(&original);
        if invalid {
            *name = format!("_{i}");
        }
        seen.push(original);
    }
}

/// Validates a single type/field name: must be a valid, non-keyword identifier.
///
/// Uses `str.isidentifier` semantics (which accept keywords syntactically), so
/// the keyword check is applied separately — matching CPython's distinct error
/// messages.
fn validate_name(name: &str) -> RunResult<()> {
    if !str_isidentifier(name) {
        Err(SimpleException::new_msg(
            ExcType::ValueError,
            format!(
                "Type names and field names must be valid identifiers: {}",
                py_repr_str(name)
            ),
        )
        .into())
    } else if is_keyword(name) {
        Err(SimpleException::new_msg(
            ExcType::ValueError,
            format!("Type names and field names cannot be a keyword: {}", py_repr_str(name)),
        )
        .into())
    } else {
        Ok(())
    }
}

/// Validates every field name (identifier + keyword checks, then no leading
/// underscore and no duplicates), matching CPython's ordering and messages.
/// When `rename` is set, the leading-underscore check is skipped (renamed
/// fields legitimately start with `_`).
fn validate_field_names(names: &[String], rename: bool) -> RunResult<()> {
    for name in names {
        validate_name(name)?;
    }
    let mut seen: Vec<&str> = Vec::with_capacity(names.len());
    for name in names {
        if name.starts_with('_') && !rename {
            return Err(SimpleException::new_msg(
                ExcType::ValueError,
                format!("Field names cannot start with an underscore: {}", py_repr_str(name)),
            )
            .into());
        }
        if seen.contains(&name.as_str()) {
            return Err(SimpleException::new_msg(
                ExcType::ValueError,
                format!("Encountered duplicate field name: {}", py_repr_str(name)),
            )
            .into());
        }
        seen.push(name);
    }
    Ok(())
}

/// Renders a Python `repr()` of a string (single-quoted) for error messages.
fn py_repr_str(s: &str) -> String {
    format!("'{s}'")
}

/// Constructs a [`NamedTuple`] instance from a call to a `NamedTupleClass`.
///
/// Binds positional and keyword arguments to the class's field names (applying
/// trailing defaults), reproducing CPython's `{typename}.__new__` arity/keyword
/// errors.
pub(crate) fn instantiate(
    vm: &mut VM<'_, impl ResourceTracker>,
    class_id: HeapId,
    args: ArgValues,
) -> RunResult<CallResult> {
    // Snapshot the class's shape (names + defaults) so the borrow is released
    // before we allocate the instance.
    let (typename, field_names, defaults) = {
        let HeapReadOutput::NamedTupleClass(class) = vm.heap.read(class_id) else {
            unreachable!("namedtuple class")
        };
        let c = class.get(vm.heap);
        let typename = c.typename.clone();
        let field_names = c.field_names.clone();
        let defaults: Vec<Value> = c.defaults.iter().map(|v| v.clone_with_heap(vm.heap)).collect();
        (typename, field_names, defaults)
    };

    let new_name = format!("{}.__new__", typename.as_str(vm.interns));
    let items = bind_fields(&new_name, &field_names, defaults, args, vm)?;
    let nt = NamedTuple::new(typename, field_names, items);
    let id = vm.heap.allocate(HeapData::NamedTuple(nt))?;
    Ok(CallResult::Value(Value::Ref(id)))
}

/// Binds call arguments to field slots, applying `defaults` to trailing fields
/// and raising CPython-matching errors for arity/keyword mistakes. Consumes
/// `defaults` and all argument values.
fn bind_fields(
    new_name: &str,
    field_names: &[EitherStr],
    defaults: Vec<Value>,
    args: ArgValues,
    vm: &mut VM<'_, impl ResourceTracker>,
) -> RunResult<Vec<Value>> {
    let n = field_names.len();
    let required = n - defaults.len();
    let (pos, kwargs) = args.into_parts();
    let pos: Vec<Value> = pos.collect();

    if pos.len() > n {
        // "takes N positional arguments but M were given" (CPython counts self).
        let given = pos.len();
        pos.drop_with_heap(vm);
        kwargs.drop_with_heap(vm);
        defaults.drop_with_heap(vm);
        return Err(ExcType::type_error_too_many_positional_range(
            new_name,
            required + 1,
            n + 1,
            given + 1,
            0,
        ));
    }

    let mut slots: Vec<Option<Value>> = Vec::with_capacity(n);
    for v in pos {
        slots.push(Some(v));
    }
    while slots.len() < n {
        slots.push(None);
    }

    // Bind keyword arguments by field name.
    for (key, value) in kwargs {
        let key_str = key.to_str(vm).map(str::to_owned).unwrap_or_default();
        match field_names.iter().position(|f| f.as_str(vm.interns) == key_str) {
            Some(idx) if slots[idx].is_none() => {
                slots[idx] = Some(value);
            }
            Some(_) => {
                // Already filled positionally → multiple values.
                value.drop_with_heap(vm);
                drop_slots(slots, vm);
                defaults.drop_with_heap(vm);
                return Err(ExcType::type_error_duplicate_arg(new_name, &key_str));
            }
            None => {
                value.drop_with_heap(vm);
                drop_slots(slots, vm);
                defaults.drop_with_heap(vm);
                return Err(ExcType::type_error_unexpected_keyword(new_name, &key_str));
            }
        }
    }

    // Fill defaults for missing trailing fields; collect any still-missing names.
    let default_start = required;
    let mut result: Vec<Value> = Vec::with_capacity(n);
    let mut missing: Vec<&str> = Vec::new();
    let mut defaults_iter = defaults.into_iter();
    let mut default_idx = 0;
    for (i, slot) in slots.into_iter().enumerate() {
        if let Some(v) = slot {
            // Advance the defaults cursor so a later defaulted slot pairs up.
            if i >= default_start {
                if let Some(unused) = defaults_iter.next() {
                    unused.drop_with_heap(vm);
                }
                default_idx += 1;
            }
            result.push(v);
        } else if i >= default_start {
            // Use the matching default (defaults align to the last fields).
            let target = i - default_start;
            while default_idx < target {
                if let Some(unused) = defaults_iter.next() {
                    unused.drop_with_heap(vm);
                }
                default_idx += 1;
            }
            match defaults_iter.next() {
                Some(d) => result.push(d),
                None => result.push(Value::None),
            }
            default_idx += 1;
        } else {
            missing.push(field_names[i].as_str(vm.interns));
        }
    }
    // Drop any defaults we never consumed.
    for leftover in defaults_iter {
        leftover.drop_with_heap(vm);
    }

    if !missing.is_empty() {
        result.drop_with_heap(vm);
        return Err(ExcType::type_error_missing_positional_with_names(new_name, &missing));
    }
    Ok(result)
}

/// Drops all filled argument slots (used on error paths).
fn drop_slots(slots: Vec<Option<Value>>, vm: &mut VM<'_, impl ResourceTracker>) {
    for v in slots.into_iter().flatten() {
        v.drop_with_heap(vm);
    }
}

impl<'h> HeapRead<'h, NamedTupleClass> {
    /// Builds the `_fields` tuple (of field-name strings).
    fn fields_tuple(&self, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Value> {
        let names: Vec<String> = self
            .get(vm.heap)
            .field_names
            .iter()
            .map(|f| f.as_str(vm.interns).to_owned())
            .collect();
        let mut items = smallvec::SmallVec::<[Value; 3]>::new();
        for name in names {
            items.push(allocate_string(name, vm.heap)?);
        }
        Ok(allocate_tuple(items, vm.heap)?)
    }

    /// Builds the `_field_defaults` dict mapping each defaulted field to its
    /// default value.
    fn field_defaults_dict(&self, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Value> {
        let (names, count) = {
            let c = self.get(vm.heap);
            let count = c.defaults.len();
            let start = c.field_names.len() - count;
            let names: Vec<String> = c.field_names[start..]
                .iter()
                .map(|f| f.as_str(vm.interns).to_owned())
                .collect();
            (names, count)
        };
        let mut pairs = Vec::with_capacity(count);
        for (i, name) in names.into_iter().enumerate() {
            let key = allocate_string(name, vm.heap)?;
            let value = self.get(vm.heap).defaults[i].clone_with_heap(vm.heap);
            pairs.push((key, value));
        }
        let dict = Dict::from_pairs(pairs, vm)?;
        let id = vm.heap.allocate(HeapData::Dict(dict))?;
        Ok(Value::Ref(id))
    }
}

impl<'h> PyTrait<'h> for HeapRead<'h, NamedTupleClass> {
    fn py_type(&self, _vm: &VM<'h, impl ResourceTracker>) -> Type {
        // A class object's own type is `type` (matching `type(Point) is type`).
        Type::Type
    }

    fn py_len(&self, _vm: &VM<'h, impl ResourceTracker>) -> Option<usize> {
        None
    }

    fn py_bool(&self, _vm: &mut VM<'h, impl ResourceTracker>) -> bool {
        true
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Option<bool>> {
        // Class objects compare by identity (handled before the heap read).
        Ok(None)
    }

    fn py_repr_fmt(
        &self,
        f: &mut impl Write,
        vm: &mut VM<'h, impl ResourceTracker>,
        _heap_ids: &mut LazyHeapSet,
    ) -> RunResult<()> {
        // CPython renders `<class '__main__.Point'>`.
        Ok(write!(f, "<class '__main__.{}'>", self.get(vm.heap).typename(vm))?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h, impl ResourceTracker>) -> RunResult<Option<CallResult>> {
        let value = match attr.static_string() {
            Some(StaticStrings::DunderName) => allocate_string(self.get(vm.heap).typename(vm).to_owned(), vm.heap)?,
            Some(StaticStrings::Fields) => self.fields_tuple(vm)?,
            Some(StaticStrings::FieldDefaults) => self.field_defaults_dict(vm)?,
            _ => {
                return Err(ExcType::attribute_error(
                    self.get(vm.heap).typename(vm),
                    attr.as_str(vm.interns),
                ));
            }
        };
        Ok(Some(CallResult::Value(value)))
    }

    fn py_call_attr(
        &mut self,
        self_id: HeapId,
        vm: &mut VM<'h, impl ResourceTracker>,
        attr: &EitherStr,
        args: ArgValues,
    ) -> RunResult<CallResult> {
        if attr.static_string() == Some(StaticStrings::Make) {
            namedtuple_make(self_id, args, vm)
        } else {
            args.drop_with_heap(vm);
            Err(ExcType::attribute_error(
                self.get(vm.heap).typename(vm),
                attr.as_str(vm.interns),
            ))
        }
    }
}

/// Implements the `_make(iterable)` classmethod: builds an instance from an
/// iterable that must have exactly `len(_fields)` items.
fn namedtuple_make(class_id: HeapId, args: ArgValues, vm: &mut VM<'_, impl ResourceTracker>) -> RunResult<CallResult> {
    let iterable = args.get_one_arg("_make", vm.heap)?;
    let items: Vec<Value> = MontyIter::new(iterable, vm)?.collect(vm)?;

    let (typename, field_names) = {
        let HeapReadOutput::NamedTupleClass(class) = vm.heap.read(class_id) else {
            unreachable!("namedtuple class")
        };
        let c = class.get(vm.heap);
        (c.typename.clone(), c.field_names.clone())
    };
    if items.len() != field_names.len() {
        let got = items.len();
        let expected = field_names.len();
        items.drop_with_heap(vm);
        return Err(
            SimpleException::new_msg(ExcType::TypeError, format!("Expected {expected} arguments, got {got}")).into(),
        );
    }
    let nt = NamedTuple::new(typename, field_names, items);
    let id = vm.heap.allocate(HeapData::NamedTuple(nt))?;
    Ok(CallResult::Value(Value::Ref(id)))
}

impl HeapItem for NamedTupleClass {
    fn py_estimate_size(&self) -> usize {
        mem::size_of::<Self>()
            + self.typename.py_estimate_size()
            + self.field_names.iter().map(EitherStr::py_estimate_size).sum::<usize>()
            + self.defaults.len() * mem::size_of::<Value>()
    }

    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        for d in &mut self.defaults {
            d.py_dec_ref_ids(stack);
        }
    }
}
