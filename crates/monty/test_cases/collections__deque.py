# Tests for collections.deque

from collections import deque

# === Construction and iteration ===
assert list(deque()) == [], 'empty deque'
assert list(deque([1, 2, 3])) == [1, 2, 3], 'deque from list'
assert list(deque((1, 2, 3))) == [1, 2, 3], 'deque from tuple'
assert list(deque('abc')) == ['a', 'b', 'c'], 'deque from str'
assert list(deque(range(3))) == [0, 1, 2], 'deque from range'
assert len(deque([1, 2, 3])) == 3, 'deque len'

# === append / appendleft ===
d = deque([1, 2, 3])
d.append(4)
assert list(d) == [1, 2, 3, 4], 'append adds to right'
d.appendleft(0)
assert list(d) == [0, 1, 2, 3, 4], 'appendleft adds to left'

# === pop / popleft ===
assert d.pop() == 4, 'pop returns rightmost'
assert d.popleft() == 0, 'popleft returns leftmost'
assert list(d) == [1, 2, 3], 'after pops'

# === indexing ===
d = deque([10, 20, 30])
assert d[0] == 10, 'index 0'
assert d[2] == 30, 'index 2'
assert d[-1] == 30, 'negative index'
assert d[-3] == 10, 'negative index full'
d[1] = 99
assert list(d) == [10, 99, 30], 'setitem'
d[-1] = 77
assert list(d) == [10, 99, 77], 'setitem negative'

# === extend / extendleft ===
d = deque([1, 2])
d.extend([3, 4])
assert list(d) == [1, 2, 3, 4], 'extend appends in order'
d.extendleft([0, -1])
assert list(d) == [-1, 0, 1, 2, 3, 4], 'extendleft reverses order'

# === insert ===
d = deque([1, 2, 4])
d.insert(2, 3)
assert list(d) == [1, 2, 3, 4], 'insert at index'
d.insert(0, 0)
assert list(d) == [0, 1, 2, 3, 4], 'insert at front'
d.insert(100, 5)
assert list(d) == [0, 1, 2, 3, 4, 5], 'insert past end appends'

# === remove ===
d = deque([1, 2, 3, 2])
d.remove(2)
assert list(d) == [1, 3, 2], 'remove first occurrence'

# === count / index ===
d = deque([1, 2, 2, 3, 2])
assert d.count(2) == 3, 'count occurrences'
assert d.count(9) == 0, 'count absent'
assert d.index(2) == 1, 'index first occurrence'
assert d.index(2, 2) == 2, 'index with start'
assert d.index(3, 0, 4) == 3, 'index with start and stop'

# === reverse ===
d = deque([1, 2, 3])
d.reverse()
assert list(d) == [3, 2, 1], 'reverse in place'

# === rotate ===
d = deque([1, 2, 3, 4, 5])
d.rotate(1)
assert list(d) == [5, 1, 2, 3, 4], 'rotate right by 1'
d.rotate(-1)
assert list(d) == [1, 2, 3, 4, 5], 'rotate left by 1'
d.rotate(2)
assert list(d) == [4, 5, 1, 2, 3], 'rotate right by 2'
d.rotate()
assert list(d) == [3, 4, 5, 1, 2], 'rotate default is 1'
d = deque([1, 2, 3])
d.rotate(7)
assert list(d) == [3, 1, 2], 'rotate wraps modulo length'

# === clear / copy ===
d = deque([1, 2, 3])
c = d.copy()
assert list(c) == [1, 2, 3], 'copy has same items'
c.append(4)
assert list(d) == [1, 2, 3], 'copy is independent'
d.clear()
assert list(d) == [], 'clear empties deque'
assert len(d) == 0, 'clear len is zero'

# === maxlen ===
assert deque([1, 2, 3]).maxlen is None, 'default maxlen is None'
b = deque([1, 2, 3], maxlen=3)
assert b.maxlen == 3, 'maxlen attribute'
b.append(4)
assert list(b) == [2, 3, 4], 'append drops from left when full'
b.appendleft(1)
assert list(b) == [1, 2, 3], 'appendleft drops from right when full'
b2 = deque(maxlen=2)
b2.extend([1, 2, 3, 4])
assert list(b2) == [3, 4], 'extend respects maxlen'
z = deque(maxlen=0)
z.append(1)
assert list(z) == [], 'maxlen 0 discards everything'
over = deque([1, 2, 3, 4, 5], maxlen=3)
assert list(over) == [3, 4, 5], 'constructor truncates to maxlen'

# === membership ===
d = deque([1, 2, 3])
assert 2 in d, 'membership present'
assert 9 not in d, 'membership absent'

# === reversed ===
assert list(reversed(deque([1, 2, 3]))) == [3, 2, 1], 'reversed deque'

# === bool ===
assert bool(deque([1])) is True, 'non-empty deque is truthy'
assert bool(deque()) is False, 'empty deque is falsy'

# === equality ===
assert deque([1, 2, 3]) == deque([1, 2, 3]), 'equal deques'
assert deque([1, 2]) != deque([2, 1]), 'order matters'
assert deque([1, 2]) != deque([1, 2, 3]), 'length matters'
assert (deque([1, 2]) == [1, 2]) is False, 'deque never equals list'

# === ordering ===
assert deque([1, 2]) < deque([1, 3]), 'less than'
assert deque([1, 2, 3]) > deque([1, 2]), 'longer is greater when prefix equal'
assert deque([1, 2]) <= deque([1, 2]), 'less than or equal'

# === repr ===
assert repr(deque([1, 2, 3])) == 'deque([1, 2, 3])', 'repr without maxlen'
assert repr(deque([1, 2], maxlen=5)) == 'deque([1, 2], maxlen=5)', 'repr with maxlen'
assert repr(deque()) == 'deque([])', 'repr empty'

# === nested references ===
d = deque([[1], [2]])
d[0].append(9)
assert list(d) == [[1, 9], [2]], 'nested list mutation'

# === error cases ===
try:
    deque().pop()
    assert False, 'expected pop from empty to raise'
except IndexError as e:
    assert str(e) == 'pop from an empty deque', 'pop empty message'

try:
    deque().popleft()
    assert False, 'expected popleft from empty to raise'
except IndexError as e:
    assert str(e) == 'pop from an empty deque', 'popleft empty message'

try:
    deque([1, 2])[5]
    assert False, 'expected out of range to raise'
except IndexError as e:
    assert str(e) == 'deque index out of range', 'index out of range message'

try:
    deque([1, 2])['a']
    assert False, 'expected non-int index to raise'
except TypeError as e:
    assert str(e) == "sequence index must be integer, not 'str'", 'str index message'

try:
    deque([1, 2])[1:2]
    assert False, 'expected slice index to raise'
except TypeError as e:
    assert str(e) == "sequence index must be integer, not 'slice'", 'slice index message'

try:
    deque(maxlen=-1)
    assert False, 'expected negative maxlen to raise'
except ValueError as e:
    assert str(e) == 'maxlen must be non-negative', 'negative maxlen message'

try:
    deque([1, 2]).index(9)
    assert False, 'expected index of absent to raise'
except ValueError as e:
    assert str(e) == '9 is not in deque', 'index absent message'

try:
    deque(['x']).remove('y')
    assert False, 'expected remove of absent to raise'
except ValueError as e:
    assert str(e) == "'y' is not in deque", 'remove absent message'

try:
    full = deque([1, 2, 3], maxlen=3)
    full.insert(0, 9)
    assert False, 'expected insert on full bounded deque to raise'
except IndexError as e:
    assert str(e) == 'deque already at its maximum size', 'insert full message'

# === mutation during iteration ===
try:
    d = deque([1, 2, 3])
    for x in d:
        d.append(x)
    assert False, 'expected mutation during iteration to raise'
except RuntimeError as e:
    assert str(e) == 'deque mutated during iteration', 'mutation message'
