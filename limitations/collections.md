# `collections`

Monty implements the most commonly used members of `collections`. Only the
following names are importable; everything else in CPython's `collections`
(`OrderedDict`, `ChainMap`, `UserDict`, `UserList`, `UserString`, and the
`collections.abc` re-exports) is **not** provided and fails at type-check time
(the custom stub only exposes the implemented names) and at runtime:

- `deque`

The dict-like members (`defaultdict`, `Counter`) and `namedtuple` are
documented in their own sections below as they land.

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
