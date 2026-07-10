# === float modulo: result takes the divisor's sign ===
assert 7.5 % 2 == 1.5, 'positive float % positive int'
assert -7.0 % 3.0 == 2.0, 'negative dividend, positive divisor'
assert 7.0 % -3.0 == -2.0, 'positive dividend, negative divisor'
assert -7.0 % -3.0 == -1.0, 'negative dividend, negative divisor'
assert -7 % 3.0 == 2.0, 'int % float sign'
assert -7.0 % 3 == 2.0, 'float % int sign'
assert str(-6.0 % 3.0) == '0.0', 'zero result takes positive divisor sign'
assert str(6.0 % -3.0) == '-0.0', 'zero result takes negative divisor sign'

# === infinite divisor ===
assert 5.0 % float('inf') == 5.0, 'positive % inf is identity'
assert -5.0 % float('inf') == float('inf'), 'negative % inf is inf'
assert 5.0 % -float('inf') == -float('inf'), 'positive % -inf is -inf'

# === `%` agrees with divmod's remainder ===
assert divmod(-7.0, 3.0)[1] == -7.0 % 3.0, 'divmod remainder matches %'
assert divmod(7.0, -3.0)[1] == 7.0 % -3.0, 'divmod remainder matches % (negative divisor)'

# === fused `x % n == k` comparison (int-literal k) matches unfused ===
assert (-7.0 % 3.0 == 2) is True, 'fused negative float mod'
assert (7.0 % -3.0 == -2) is True, 'fused negative divisor'
assert (5.0 % 3.0 == 2) is True, 'fused positive float mod'

# === fused comparison raises the same TypeError as unfused `%` ===
try:
    [1] % 2 == 0
    assert False, 'expected TypeError from fused %'
except TypeError as e:
    assert str(e) == "unsupported operand type(s) for %: 'list' and 'int'", 'fused % TypeError message'

# === zero divisor raises, including in the fused `== k` form ===
for a, b in [(5.0, 0.0), (5.0, 0), (5, 0.0), (5, 0)]:
    try:
        a % b
        assert False, 'expected ZeroDivisionError from %'
    except ZeroDivisionError as e:
        assert str(e) == 'division by zero', 'plain % zero divisor message'
    try:
        a % b == 1
        assert False, 'expected ZeroDivisionError from fused == comparison'
    except ZeroDivisionError as e:
        assert str(e) == 'division by zero', 'fused == zero divisor message'
