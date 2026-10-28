# Tests for object.__new__, Cls.__new__, and ExcType.__new__ safety and descriptor parity.


# === object.__new__ rejects built-in and user exception classes ===
for exc_cls in (BaseException, Exception, ValueError, KeyError):
    try:
        object.__new__(exc_cls)
        assert False, f'expected TypeError for {exc_cls}'
    except TypeError as err:
        assert str(err) == f'object.__new__({exc_cls.__name__}) is not safe, use {exc_cls.__name__}.__new__()'


class CustomErr(ValueError):
    pass


try:
    object.__new__(CustomErr)
    assert False, 'expected TypeError for CustomErr'
except TypeError as err:
    assert str(err) == 'object.__new__(CustomErr) is not safe, use CustomErr.__new__()'


# === object.__new__ excess-argument rules ===
class Bare:
    pass


try:
    object.__new__(Bare, 1)
    assert False, 'expected TypeError for Bare excess pos arg'
except TypeError as err:
    assert str(err) == 'Bare() takes no arguments'

try:
    object.__new__(Bare, k=1)
    assert False, 'expected TypeError for Bare excess kwarg'
except TypeError as err:
    assert str(err) == 'Bare() takes no arguments'


class OverridesInit:
    def __init__(self, x, y=2):
        self.x = x
        self.y = y


raw = object.__new__(OverridesInit, 10, y=20)
assert isinstance(raw, OverridesInit)
assert not hasattr(raw, 'x')


class OverridesNew:
    def __new__(cls, *args, **kwargs):
        return object.__new__(cls)


try:
    object.__new__(OverridesNew, 1)
    assert False, 'expected TypeError when __new__ is overridden'
except TypeError as err:
    assert str(err) == 'object.__new__() takes exactly one argument (the type to instantiate)'

try:
    object.__new__(OverridesNew, k=1)
    assert False, 'expected TypeError when __new__ is overridden with kwargs'
except TypeError as err:
    assert str(err) == 'object.__new__() takes exactly one argument (the type to instantiate)'


# === Descriptor / getattr parity for __new__, __init__, and mro ===
fn_bare_new = Bare.__new__
b_inst = fn_bare_new(Bare)
assert isinstance(b_inst, Bare)
assert Bare.__init__(b_inst) is None
assert b_inst.__init__() is None
fn_mro = Bare.mro
assert fn_mro() == [Bare, object]

fn_ve_new = ValueError.__new__
ce1 = fn_ve_new(CustomErr, 'msg', 42)
assert isinstance(ce1, CustomErr) and ce1.args == ('msg', 42)

fn_ce_new = CustomErr.__new__
ce2 = fn_ce_new(CustomErr, 'second')
assert isinstance(ce2, CustomErr) and ce2.args == ('second',)

fn_ce_init = CustomErr.__init__
fn_ce_init(ce2, 'reinit', 99)
assert ce2.args == ('reinit', 99)

for bad_call, expected_msg in [
    (lambda: ValueError.__new__(), 'ValueError.__new__(): not enough arguments'),
    (lambda: ValueError.__new__(123), 'ValueError.__new__(X): X is not a type object (int)'),
    (
        lambda: ValueError.__new__(TypeError),
        'ValueError.__new__(TypeError): TypeError is not a subtype of ValueError',
    ),
    (
        lambda: ValueError.__new__(Bare),
        'ValueError.__new__(Bare): Bare is not a subtype of ValueError',
    ),
]:
    try:
        bad_call()
        assert False, f'expected TypeError: {expected_msg}'
    except TypeError as err:
        assert str(err) == expected_msg, f'{err!r} != {expected_msg!r}'
