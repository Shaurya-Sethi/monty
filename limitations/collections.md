# `collections`

Monty implements the most commonly used members of `collections`. Only the
following names are importable; everything else in CPython's `collections`
(`OrderedDict`, `ChainMap`, `UserDict`, `UserList`, `UserString`, and the
`collections.abc` re-exports) is **not** provided and fails at type-check time
(the custom stub only exposes the implemented names) and at runtime:

- `deque`
- `defaultdict`
- `Counter`
- `namedtuple`

The type checker uses a Monty-narrowed `collections` stub
(`crates/monty-typeshed/custom/collections/__init__.pyi`) exposing only these
members, so referencing an unimplemented member (e.g. `collections.OrderedDict`)
is a type error as well as a runtime `AttributeError`.

**`collections.abc` is not importable at runtime.** `from collections.abc
import ...` type-checks (the ABCs are used only as annotations) but raises
`ModuleNotFoundError` if executed. Use the names only in annotations / under
`if TYPE_CHECKING:`.

### General

- **`type(obj).__name__` returns the qualified name.** For all three types
  `type(x).__name__` is `"collections.defaultdict"` / `"collections.Counter"` /
  `"collections.deque"` rather than the bare `"defaultdict"` / `"Counter"` /
  `"deque"`. This is a pre-existing Monty behavior for qualified builtin types
  (e.g. `datetime.datetime.__name__` is likewise qualified). `str(type(x))`
  (`<class 'collections.defaultdict'>`) matches CPython.
- **No subclassing** of these types, and no `copy.copy` / `pickle` support (the
  `copy` and `pickle` modules are not importable); use the `.copy()` method.

## `deque`

Supported: construction from any iterable, `maxlen`, `append`, `appendleft`,
`pop`, `popleft`, `extend`, `extendleft`, `insert`, `remove`, `clear`, `copy`,
`count`, `index` (with optional `start`/`stop`), `reverse`, `rotate`, integer
indexing and assignment, iteration, `reversed()`, `in`, `len()`, `==`/`!=`,
ordering (`<`, `<=`, `>`, `>=`), `bool()`, and `repr()`.

Divergences from CPython:

- **No `+`, `*`, `+=`, or `*=`.** Deque concatenation (`d1 + d2`) and repetition
  (`d * n`) raise `TypeError: unsupported operand type(s)`. Use `.extend()` /
  `.extendleft()` instead. (CPython supports all four.)
- **No slicing.** `d[1:2]` raises `TypeError: sequence index must be integer,
  not 'slice'` (CPython raises the same error — deque genuinely does not support
  slices — but note the deque also has no `__getitem__` slice fallback here).
- **No `del d[i]`.** Item deletion by index is not supported (the `del`
  statement is unimplemented Monty-wide, not deque-specific).
- **No `__reversed__`-based reverse iterator object.** `reversed(d)` still works
  (it goes through indexing + length), but there is no distinct
  `_collections._deque_reverse_iterator` type.
- **No `__copy__` / pickling / `__reduce__`.** `copy.copy(d)` and pickling are
  unavailable (the `copy` and `pickle` modules are not importable anyway); use
  the `.copy()` method.
- **`maxlen` argument coercion.** A non-integer `maxlen` is reported by the
  constructor body; a `maxlen` that overflows the platform integer range is not
  specially handled.

## `defaultdict`

Supported: `default_factory` (attribute), missing-key insertion via the factory,
construction from a mapping/iterable plus keyword arguments, all standard `dict`
methods (`get`, `keys`, `values`, `items`, `pop`, `popitem`, `setdefault`,
`update`, `clear`, `copy`), iteration, membership, equality with plain dicts,
`repr`, and `bool`.

Divergences from CPython:

- **`default_factory` must be a builtin callable.** On a missing key the factory
  is only invoked for builtin callables (`int`, `list`, `set`, `dict`, `tuple`,
  `str`, `float`, `bool`, `frozenset`, …). A **user-defined** factory (a `lambda`
  or `def` function) constructs fine but raises
  `TypeError: defaultdict with a non-builtin default_factory is not supported in
  Monty` on the missing-key access. This is because `__getitem__` cannot invoke a
  Python callback that needs its own VM frame. `.get()`, explicit assignment, and
  iteration are unaffected.
- **`default_factory` is not reassignable** after construction (`dd.default_factory
  = f` is unsupported).

## `Counter`

Supported: construction from an iterable (tallying), a mapping, or keyword
arguments; `c[missing]` returning `0` without inserting; `most_common([n])`;
`elements()`; `update()`; `subtract()`; the arithmetic operators `+`, `-`, `&`,
`|` (each dropping non-positive results); all standard `dict` methods; iteration;
equality with plain dicts; count-descending `repr`; and `bool`.

Divergences from CPython:

- **`elements()` returns a `list`, not a lazy iterator.** CPython returns an
  `itertools.chain` object; Monty materializes a `list`. Iterating or calling
  `list(...)` on the result is identical, but `type(c.elements())` differs.
- **No in-place operators** `+=`, `-=`, `&=`, `|=` between two Counters (the
  binary forms are supported; use `.update()` / `.subtract()` for in-place count
  changes).
- **No unary `+c` / `-c`** (which in CPython keep positive / negative counts).
- **`total()`, `fromkeys`** are not implemented (`Counter.fromkeys` raises in
  CPython too, but `total()` is missing here).
- **Counts are integers.** Arithmetic assumes integer counts; non-integer values
  are treated as `0` for count purposes rather than raising.

## `namedtuple`

`collections.namedtuple(typename, field_names, *, rename=False, defaults=None,
module=None)` builds a callable class producing named-tuple instances. Field
names may be a whitespace/comma-separated string or an iterable of strings, and
are validated (identifiers, non-keywords, no leading underscore, no duplicates)
with CPython-matching `ValueError`s; `rename=True` auto-fixes invalid names to
`_0`, `_1`, ….

See [namedtuple.md](namedtuple.md) for the full instance surface and
divergences. The headline divergence: **`type(instance) is TheClass` is
`False`** and `type(instance).__name__` is `'namedtuple'` — the class object
carries the correct name/`_fields`/repr, but the runtime type-identity link
between an instance and its specific class is not established. The `module`
argument is accepted and ignored (Monty has no per-namedtuple module objects,
so the class repr is always `<class '__main__.Name'>`).
