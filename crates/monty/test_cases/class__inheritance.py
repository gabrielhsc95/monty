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

# === Special "dunder" method inheritance ===

class ContainerBase:
    def __init__(self, data):
        self.data = data

    def __str__(self):
        return 'Container(' + str(self.data) + ')'

    def __eq__(self, other):
        if type(self) is type(other):
            return self.data == other.data
        return False

    def __contains__(self, item):
        return item in self.data

    def __iter__(self):
        return iter(self.data)


class MyContainer(ContainerBase):
    def is_empty(self):
        return len(self.data) == 0


c1 = MyContainer([1, 2, 3])
c2 = MyContainer([1, 2, 3])
c3 = MyContainer([4, 5])

assert str(c1) == 'Container([1, 2, 3])'
assert c1 == c2
assert not (c1 == c3)
assert 2 in c1
assert 9 not in c1

items = []
for x in c3:
    items.append(x)
assert items == [4, 5]
assert not c1.is_empty()

# Test object ordering in bases
class ObjBase:
    pass


class ValidObjSub(ObjBase, object):
    pass


assert isinstance(ValidObjSub(), ObjBase)

try:
    class InvalidObjSub(object, ObjBase):
        pass

    assert False, "expected MRO conflict with object first"
except TypeError as e:
    assert str(e) == "Cannot create a consistent method resolution order (MRO) for bases object, ObjBase"

class ObjA:
    pass


class ObjB:
    pass


try:
    class InvalidMidObjSub(ObjA, object, ObjB):
        pass

    assert False, "expected MRO conflict with object in middle"
except TypeError as e:
    assert str(e) == "Cannot create a consistent method resolution order (MRO) for bases object, ObjB"

print("success")


