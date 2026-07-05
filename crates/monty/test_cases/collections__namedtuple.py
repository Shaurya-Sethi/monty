# Tests for collections.namedtuple

from collections import namedtuple

# === Construction and field access ===
Point = namedtuple('Point', ['x', 'y'])
p = Point(1, 2)
assert p.x == 1, 'attribute access x'
assert p.y == 2, 'attribute access y'
assert p[0] == 1, 'index access 0'
assert p[1] == 2, 'index access 1'
assert p[-1] == 2, 'negative index'
assert len(p) == 2, 'length'

# === field_names as a space/comma string ===
P = namedtuple('P', 'a b c')
assert P._fields == ('a', 'b', 'c'), 'space-separated fields'
Pc = namedtuple('Pc', 'a, b, c')
assert Pc._fields == ('a', 'b', 'c'), 'comma-separated fields'

# === keyword construction ===
assert Point(x=1, y=2) == Point(1, 2), 'keyword construction'
assert Point(1, y=2) == Point(1, 2), 'mixed positional and keyword'

# === _fields (class and instance) ===
assert Point._fields == ('x', 'y'), 'class _fields'
assert p._fields == ('x', 'y'), 'instance _fields'

# === _make ===
assert Point._make([3, 4]) == Point(3, 4), '_make from list'
assert Point._make((5, 6)) == Point(5, 6), '_make from tuple'

# === _asdict ===
assert p._asdict() == {'x': 1, 'y': 2}, '_asdict returns ordered dict'

# === _replace ===
assert p._replace(x=10) == Point(10, 2), '_replace one field'
assert p._replace(x=10, y=20) == Point(10, 20), '_replace both fields'
assert p == Point(1, 2), '_replace does not mutate original'

# === defaults ===
D = namedtuple('D', 'a b c', defaults=[10, 20])
assert D(1) == D(1, 10, 20), 'trailing defaults applied'
assert D(1, 2) == D(1, 2, 20), 'partial defaults'
assert D(1, 2, 3) == D(1, 2, 3), 'all provided'
assert D._field_defaults == {'b': 10, 'c': 20}, '_field_defaults'
assert namedtuple('E', 'a b').__name__ == 'E', 'class __name__'

# === Tuple compatibility ===
assert p == (1, 2), 'equals plain tuple'
assert (1, 2) == p, 'plain tuple equals namedtuple'
assert isinstance(p, tuple), 'namedtuple is a tuple'
assert list(p) == [1, 2], 'iterable / list()'
a, b = p
assert (a, b) == (1, 2), 'sequence unpacking'
assert [v for v in p] == [1, 2], 'comprehension iteration'

# === Inherited tuple methods ===
T = namedtuple('T', 'a b c d')
t = T(1, 2, 2, 3)
assert t.count(2) == 2, 'inherited count'
assert t.index(2) == 1, 'inherited index'
assert t.index(3) == 3, 'inherited index later'

# === Hashing (usable as dict key / set member) ===
d = {Point(1, 2): 'a'}
assert d[Point(1, 2)] == 'a', 'namedtuple as dict key'
assert Point(1, 2) in {Point(1, 2)}, 'namedtuple as set member'

# === Nested namedtuples ===
Line = namedtuple('Line', 'start end')
line = Line(Point(0, 0), Point(1, 1))
assert line.start.x == 0, 'nested field access'
assert line.end.y == 1, 'nested field access 2'
assert repr(line) == 'Line(start=Point(x=0, y=0), end=Point(x=1, y=1))', 'nested repr'

# === repr ===
assert repr(Point(1, 2)) == 'Point(x=1, y=2)', 'instance repr'
assert repr(namedtuple('Q', 'x')(5)) == 'Q(x=5)', 'single-field repr'

# === rename ===
R = namedtuple('R', 'x def 1y x', rename=True)
assert R._fields == ('x', '_1', '_2', '_3'), 'rename fixes invalid fields'

# === bool ===
assert bool(namedtuple('Z', 'a')(0)) is True, 'non-empty namedtuple is truthy'

# === Construction errors ===
try:
    Point(1)
    assert False, 'expected missing-argument error'
except TypeError as e:
    assert str(e) == "Point.__new__() missing 1 required positional argument: 'y'", 'missing arg message'

try:
    Point(1, 2, 3)
    assert False, 'expected too-many-arguments error'
except TypeError as e:
    assert str(e) == 'Point.__new__() takes 3 positional arguments but 4 were given', 'too many args message'

try:
    Point(1, x=2)
    assert False, 'expected multiple-values error'
except TypeError as e:
    assert str(e) == "Point.__new__() got multiple values for argument 'x'", 'multiple values message'

try:
    Point(1, 2, z=3)
    assert False, 'expected unexpected-keyword error'
except TypeError as e:
    assert str(e) == "Point.__new__() got an unexpected keyword argument 'z'", 'unexpected keyword message'

# === Factory errors ===
try:
    namedtuple('P', 'x x')
    assert False, 'expected duplicate-field error'
except ValueError as e:
    assert str(e) == "Encountered duplicate field name: 'x'", 'duplicate field message'

try:
    namedtuple('P', 'def')
    assert False, 'expected keyword-field error'
except ValueError as e:
    assert str(e) == "Type names and field names cannot be a keyword: 'def'", 'keyword field message'

try:
    namedtuple('1P', 'x')
    assert False, 'expected invalid-identifier error'
except ValueError as e:
    assert str(e) == "Type names and field names must be valid identifiers: '1P'", 'invalid identifier message'

try:
    namedtuple('P', '_x')
    assert False, 'expected leading-underscore error'
except ValueError as e:
    assert str(e) == "Field names cannot start with an underscore: '_x'", 'leading underscore message'

try:
    p._replace(z=9)
    assert False, 'expected unexpected-field _replace error'
except ValueError as e:
    assert str(e) == "Got unexpected field names: ['z']", '_replace unexpected field message'

try:
    Point._make([1])
    assert False, 'expected _make arity error'
except TypeError as e:
    assert str(e) == 'Expected 2 arguments, got 1', '_make arity message'
