# Tests for importing the collections module and its members

import collections

# === Module attribute access ===
d = collections.deque([1, 2, 3])
assert list(d) == [1, 2, 3], 'collections.deque'
dd = collections.defaultdict(int)
dd['a'] += 1
assert dd == {'a': 1}, 'collections.defaultdict'
c = collections.Counter('aab')
assert c == {'a': 2, 'b': 1}, 'collections.Counter'
Point = collections.namedtuple('Point', 'x y')
assert Point(1, 2).x == 1, 'collections.namedtuple'

# === from collections import ... ===
from collections import deque, defaultdict, Counter, namedtuple

assert list(deque([4, 5])) == [4, 5], 'imported deque'
assert defaultdict(list)['k'] == [], 'imported defaultdict'
assert Counter([1, 1, 2]) == {1: 2, 2: 1}, 'imported Counter'
NT = namedtuple('NT', ['a', 'b'])
assert NT(a=1, b=2) == (1, 2), 'imported namedtuple'
