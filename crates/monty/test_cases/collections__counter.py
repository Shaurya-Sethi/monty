# Tests for collections.Counter

from collections import Counter

# === Construction from an iterable (tallying) ===
c = Counter('aabbbc')
assert c == {'a': 2, 'b': 3, 'c': 1}, 'counts characters'
assert Counter([1, 1, 2, 3, 3, 3]) == {1: 2, 2: 1, 3: 3}, 'counts list elements'
assert Counter() == {}, 'empty Counter'

# === Construction from a mapping or kwargs ===
assert Counter({'a': 2, 'b': 3}) == {'a': 2, 'b': 3}, 'from mapping uses values as counts'
assert Counter(a=2, b=3) == {'a': 2, 'b': 3}, 'from kwargs'
assert Counter({'a': 2}, b=3) == {'a': 2, 'b': 3}, 'mapping plus kwargs'

# === Missing keys return 0 without inserting ===
c = Counter('aab')
assert c['z'] == 0, 'missing key returns 0'
assert 'z' not in c, 'missing-key access does not insert'
assert len(c) == 2, 'length unchanged after missing access'

# === most_common ===
c = Counter('aaabbc')
assert c.most_common() == [('a', 3), ('b', 2), ('c', 1)], 'most_common all, count-descending'
assert c.most_common(2) == [('a', 3), ('b', 2)], 'most_common top-n'
assert c.most_common(0) == [], 'most_common zero'
# ties preserve insertion order
ct = Counter()
ct['x'] = 1
ct['y'] = 1
ct['z'] = 1
assert ct.most_common() == [('x', 1), ('y', 1), ('z', 1)], 'ties keep insertion order'

# === elements ===
c = Counter('abcabc')
assert list(c.elements()) == ['a', 'a', 'b', 'b', 'c', 'c'], 'elements repeats in insertion order'
# non-positive counts are skipped
cz = Counter(a=2, b=0, c=-1)
assert list(cz.elements()) == ['a', 'a'], 'elements skips non-positive counts'
assert list(Counter().elements()) == [], 'elements of empty Counter'

# === update (adds counts) ===
cu = Counter(a=1)
cu.update(Counter(a=2, b=1))
assert cu == {'a': 3, 'b': 1}, 'update from Counter adds'
cu.update('aa')
assert cu == {'a': 5, 'b': 1}, 'update from iterable adds'
cu.update({'c': 4})
assert cu == {'a': 5, 'b': 1, 'c': 4}, 'update from mapping adds'
cu.update(b=10)
assert cu == {'a': 5, 'b': 11, 'c': 4}, 'update from kwargs adds'

# === subtract (may go zero or negative) ===
cs = Counter(a=5)
cs.subtract(Counter(a=2, b=3))
assert cs == {'a': 3, 'b': -3}, 'subtract keeps negatives'
cs2 = Counter(a=1)
cs2.subtract('aabb')
assert cs2 == {'a': -1, 'b': -2}, 'subtract from iterable'

# === Arithmetic operators (drop non-positive) ===
assert Counter(a=3, b=1) + Counter(a=1, c=1) == Counter({'a': 4, 'b': 1, 'c': 1}), 'addition'
assert Counter(a=3, b=1) - Counter(a=1, b=2) == Counter({'a': 2}), 'subtraction drops non-positive'
assert Counter(a=3, b=1) & Counter(a=1, b=2) == Counter({'a': 1, 'b': 1}), 'intersection is min'
assert Counter(a=3, b=1) | Counter(a=1, b=2) == Counter({'a': 3, 'b': 2}), 'union is max'
assert Counter(a=1) + Counter(a=-1) == Counter(), 'addition to zero drops key'

# === Equality with plain dict ===
assert Counter(a=1, b=2) == {'a': 1, 'b': 2}, 'Counter equals plain dict'
assert {'a': 1, 'b': 2} == Counter(a=1, b=2), 'plain dict equals Counter'

# === copy preserves subclass ===
cc = Counter('aab').copy()
assert cc == {'a': 2, 'b': 1}, 'copy has same counts'
assert cc['z'] == 0, 'copy still returns 0 for missing'
ccorig = Counter(a=1)
cccopy = ccorig.copy()
cccopy['b'] = 5
assert ccorig == {'a': 1}, 'copy is independent'

# === Standard dict methods ===
c = Counter('aab')
assert sorted(c.keys()) == ['a', 'b'], 'keys view'
assert sorted(c.values()) == [1, 2], 'values view'
assert dict(c.items()) == {'a': 2, 'b': 1}, 'items view'
assert c.get('a') == 2, 'get existing'
assert c.get('z') is None, 'get missing returns None (not 0)'
c['a'] = 10
assert c['a'] == 10, 'setitem'
del_count = c.pop('a')
assert del_count == 10, 'pop existing'
assert 'a' not in c, 'state after pop'

# === Iteration ===
c = Counter('aabbbc')
assert sorted(c) == ['a', 'b', 'c'], 'iterating yields keys'
assert dict(c) == {'a': 2, 'b': 3, 'c': 1}, 'dict() copies mapping'

# === bool ===
assert bool(Counter()) is False, 'empty Counter is falsy'
assert bool(Counter(a=1)) is True, 'non-empty is truthy'

# === repr (count-descending) ===
assert repr(Counter('aabbbc')) == "Counter({'b': 3, 'a': 2, 'c': 1})", 'repr sorts by count desc'
assert repr(Counter()) == 'Counter()', 'empty repr'
assert repr(Counter(a=1)) == "Counter({'a': 1})", 'single-entry repr'
