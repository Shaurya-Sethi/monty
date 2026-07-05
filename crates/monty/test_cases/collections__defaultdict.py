# Tests for collections.defaultdict

from collections import defaultdict

# === Basic factory behavior ===
d = defaultdict(int)
assert d['a'] == 0, 'missing key uses int factory'
assert d == {'a': 0}, 'missing-key access inserts the default'
d['b'] += 1
assert d['b'] == 1, 'augmented assignment on missing key'
assert d == {'a': 0, 'b': 1}, 'state after augmented assignment'

# === list factory ===
dl = defaultdict(list)
dl['x'].append(1)
dl['x'].append(2)
dl['y'].append(3)
assert dl == {'x': [1, 2], 'y': [3]}, 'list factory accumulates'

# === set factory ===
ds = defaultdict(set)
ds['k'].add(1)
ds['k'].add(2)
ds['k'].add(1)
assert ds['k'] == {1, 2}, 'set factory dedupes'

# === dict factory (nested) ===
dd = defaultdict(dict)
dd['a']['x'] = 1
assert dd == {'a': {'x': 1}}, 'dict factory nests'

# === default_factory attribute ===
assert defaultdict(int).default_factory is int, 'default_factory is int'
assert defaultdict(list).default_factory is list, 'default_factory is list'
assert defaultdict().default_factory is None, 'no factory means None'

# === None factory raises KeyError ===
dn = defaultdict()
try:
    dn['missing']
    assert False, 'expected KeyError with no factory'
except KeyError as e:
    assert str(e) == "'missing'", 'KeyError message is the key repr'

# === get() does not use the factory or insert ===
dg = defaultdict(int)
assert dg.get('missing') is None, 'get returns None for missing'
assert dg.get('missing', 5) == 5, 'get returns provided default'
assert 'missing' not in dg, 'get does not insert'
assert len(dg) == 0, 'get leaves defaultdict empty'

# === Construction with a mapping and kwargs ===
dm = defaultdict(int, {'a': 1})
assert dm == {'a': 1}, 'defaultdict from mapping'
dmk = defaultdict(int, {'a': 1}, b=2)
assert dmk == {'a': 1, 'b': 2}, 'defaultdict from mapping + kwargs'
di = defaultdict(int, [('a', 1), ('b', 2)])
assert di == {'a': 1, 'b': 2}, 'defaultdict from iterable of pairs'

# === Equality with plain dict ===
assert defaultdict(int, {'a': 1}) == {'a': 1}, 'defaultdict equals plain dict'
assert {'a': 1} == defaultdict(int, {'a': 1}), 'plain dict equals defaultdict'

# === copy preserves subclass, factory, and contents ===
cp = defaultdict(int, {'a': 1}).copy()
assert cp == {'a': 1}, 'copy has same items'
assert cp.default_factory is int, 'copy preserves factory'
assert cp['new'] == 0, 'copy factory still works'
cp2 = defaultdict(int, {'a': 1})
cp2copy = cp2.copy()
cp2copy['z'] = 9
assert cp2 == {'a': 1}, 'copy is independent of original'

# === Standard dict methods ===
dmeth = defaultdict(int, {'a': 1, 'b': 2})
assert sorted(dmeth.keys()) == ['a', 'b'], 'keys view'
assert sorted(dmeth.values()) == [1, 2], 'values view'
assert dict(dmeth.items()) == {'a': 1, 'b': 2}, 'items view'
assert dmeth.pop('a') == 1, 'pop existing key'
assert dmeth == {'b': 2}, 'state after pop'
dmeth.setdefault('c', 3)
assert dmeth['c'] == 3, 'setdefault inserts'
dmeth.update({'d': 4})
assert dmeth == {'b': 2, 'c': 3, 'd': 4}, 'update from mapping'
dmeth.clear()
assert dmeth == {}, 'clear empties'

# === Iteration ===
diter = defaultdict(int)
diter['a'] = 1
diter['b'] = 2
assert sorted(diter) == ['a', 'b'], 'iterating yields keys'
assert sorted(list(diter)) == ['a', 'b'], 'list() yields keys'
assert dict(diter) == {'a': 1, 'b': 2}, 'dict() copies mapping'

# === bool ===
assert bool(defaultdict(int)) is False, 'empty defaultdict is falsy'
assert bool(defaultdict(int, {'a': 1})) is True, 'non-empty is truthy'

# === repr ===
assert repr(defaultdict(int)) == "defaultdict(<class 'int'>, {})", 'empty int repr'
assert repr(defaultdict(int, {'a': 1})) == "defaultdict(<class 'int'>, {'a': 1})", 'int repr with item'
assert repr(defaultdict(list, {'x': [1]})) == "defaultdict(<class 'list'>, {'x': [1]})", 'list repr'
assert repr(defaultdict()) == 'defaultdict(None, {})', 'no-factory repr'

# === Constructor errors ===
try:
    defaultdict(5)
    assert False, 'expected non-callable factory to raise'
except TypeError as e:
    assert str(e) == 'first argument must be callable or None', 'non-callable factory message'
