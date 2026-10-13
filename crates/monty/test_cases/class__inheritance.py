# Test deep inheritance
class Grandparent:
    x = 1

class Parent(Grandparent):
    y = 2

class Child(Parent):
    z = 3

c = Child()
assert c.x == 1
assert c.y == 2
assert c.z == 3
assert isinstance(c, Child)
assert isinstance(c, Parent)
assert isinstance(c, Grandparent)

# Test multiple inheritance (Left-to-right C3 resolution)
class A:
    name = "A"
    shared = "from A"

class B:
    name = "B"
    shared = "from B"
    b_only = "only B"

class C(A, B):
    name = "C"

obj = C()
assert obj.name == "C"       # From C
assert obj.shared == "from A" # Left-most base class wins
assert obj.b_only == "only B" # Found in B
assert isinstance(obj, C)
assert isinstance(obj, A)
assert isinstance(obj, B)

# Test diamond inheritance with C3 MRO
class DiamondBase:
    val = "root"

class DiamondLeft(DiamondBase):
    pass

class DiamondRight(DiamondBase):
    val = "right"

class DiamondBottom(DiamondLeft, DiamondRight):
    pass

d = DiamondBottom()
assert d.val == "right"
assert isinstance(d, DiamondBottom)
assert isinstance(d, DiamondLeft)
assert isinstance(d, DiamondRight)
assert isinstance(d, DiamondBase)

# Test inheriting explicitly from object
class WithObject(object):
    greeting = "hi"

wo = WithObject()
assert wo.greeting == "hi"
assert isinstance(wo, WithObject)

# Test inheritance inside an enclosing function scope
def make_child():
    class LocalBase:
        x = 42

    class LocalChild(LocalBase):
        y = 100

    return LocalChild()

lc = make_child()
assert lc.x == 42
assert lc.y == 100

# Test the mock BaseModel pattern
class MockBaseModel:
    def __init__(self, **kwargs):
        for key, value in kwargs.items():
            setattr(self, key, value)

class Input(MockBaseModel):
    limit = 20

payload = {"limit": 50, "extra": "data"}
obj = Input(**payload)

assert obj.limit == 50
assert obj.extra == "data"
assert isinstance(obj, Input)
assert isinstance(obj, MockBaseModel)

# Test duplicate base rejection
class DupBase:
    pass

try:
    class DupSub(DupBase, DupBase):
        pass
    assert False, "expected duplicate base TypeError"
except TypeError as e:
    assert str(e) == "duplicate base class DupBase"

try:
    class DupObjSub(object, object):
        pass
    assert False, "expected duplicate object base TypeError"
except TypeError as e:
    assert str(e) == "duplicate base class object"

# Test MRO conflict error message with base class names
class MroX:
    pass

class MroY(MroX):
    pass

try:
    class MroConflict(MroX, MroY):
        pass
    assert False, "expected MRO conflict TypeError"
except TypeError as e:
    assert str(e) == "Cannot create a consistent method resolution order (MRO) for bases MroX, MroY"

print("success")

