# Named tuples

Named tuples can be created via `collections.namedtuple(...)` (see
[collections.md](collections.md)). They also enter the sandbox as
`sys.version_info` and as values passed in from the host via the `MontyObject`
API.

`typing.NamedTuple` still exists as a marker only: subscripting / `class`
inheritance does not produce a type (no `class` statement; see
[language.md](language.md)). There is no bare builtin `namedtuple` factory —
it must be imported from `collections`.

## Supported operations

- Construction via a `collections.namedtuple` class (positional, keyword, and
  trailing defaults).
- Indexing by integer: `nt[0]`. `IndexError` on out-of-range.
- Field access by name as an attribute: `nt.major`. `AttributeError` on
  unknown names.
- `len(nt)`, iteration (`for x in nt`), sequence unpacking (`a, b = nt`).
- Equality: `nt == nt2` and `nt == (a, b, c)` — a named tuple equals a
  plain tuple with the same elements (matches CPython).
- `isinstance(nt, tuple)` is `True`.
- Hashing: same hash as a plain tuple with the same elements; usable as
  a dict key or set element.
- `repr(nt)` — `Name(field1=v1, field2=v2, ...)` matching CPython.
- `bool(nt)` — `True` if non-empty, `False` if empty (tuple semantics).
- Named-tuple methods: `._replace(**kw)`, `._asdict()`, `._make(iterable)`,
  `._fields`, and the inherited tuple methods `.count()` / `.index()`. The
  class object also exposes `_fields`, `_field_defaults`, `_make`, `__name__`.

## NOT supported / divergences

- **`type(nt)` identity.** `type(nt) is TheClass` is `False` and
  `type(nt).__name__` is `'namedtuple'` (not the class name). The class object
  itself reports the correct `__name__`/`repr`, and instances repr and behave
  correctly — only the runtime type-identity link is missing.
- Slicing: `nt[1:3]` raises — `__getitem__` only accepts integer keys.
  (CPython returns a plain tuple for slices.)
- Lookup by string key: `nt["major"]` raises `TypeError`. Use attribute access.
- Concatenation / multiplication: `nt + nt2`, `nt * 3` are not implemented.
- Subclassing, `._source`, and `._replace` with positional arguments.
