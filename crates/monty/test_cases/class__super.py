# Test cases for super() builtin and Super proxy type.

# --- Single inheritance with super() ---
class Animal:
    def __init__(self, name):
        self.name = name

    def speak(self):
        return f"{self.name} makes a sound"

class Dog(Animal):
    def __init__(self, name, breed):
        super().__init__(name)
        self.breed = breed

    def speak(self):
        parent = super().speak()
        return f"{parent} and barks"

d = Dog("Rex", "German Shepherd")
print(d.name)
print(d.breed)
print(d.speak())

# --- Diamond multiple inheritance (cooperative MRO) ---
class A:
    def greet(self):
        return ["A"]

class B(A):
    def greet(self):
        return ["B"] + super().greet()

class C(A):
    def greet(self):
        return ["C"] + super().greet()

class D(B, C):
    def greet(self):
        return ["D"] + super().greet()

d_obj = D()
print(d_obj.greet())

# --- Two-argument super(Class, self) ---
class Base:
    def foo(self):
        return "Base.foo"

class Mid(Base):
    def foo(self):
        return "Mid.foo"

class Sub(Mid):
    def foo(self):
        return "Sub.foo"

    def mid_foo(self):
        return super().foo()

    def base_foo(self):
        return super(Mid, self).foo()

s = Sub()
print(s.foo())
print(s.mid_foo())
print(s.base_foo())

# --- super(Class, Subclass) type-binding ---
class Shape:
    kind = "generic shape"

class Polygon(Shape):
    kind = "polygon"

class Triangle(Polygon):
    kind = "triangle"

print(super(Triangle, Triangle).kind)
print(super(Polygon, Triangle).kind)

# --- Introspection attributes ---
class X:
    pass

class Y(X):
    def check_super(self):
        s = super()
        print(s.__thisclass__.__name__)
        print(type(s.__self__).__name__)
        print(s.__self_class__.__name__)

y = Y()
y.check_super()

s_unbound = super(Y)
print(s_unbound.__thisclass__.__name__)
print(s_unbound.__self__)
print(s_unbound.__self_class__)

# --- repr tests ---
print(repr(super))
print(repr(super(Y)))
print(repr(super(Y, y)))
print(repr(super(X, Y)))

# --- object.__init__ via super() ---
class Leaf:
    def __init__(self):
        super().__init__()

Leaf()
print("Leaf initialized")

# --- Error conditions ---
try:
    super(1, 2)
except TypeError as e:
    print(e)

try:
    super(X, "not_an_instance")
except TypeError as e:
    print(e)

try:
    super(Y, X())
except TypeError as e:
    print(e)

try:
    super(1, 2, 3)
except TypeError as e:
    print(e)

try:
    super(x=1)
except TypeError as e:
    print(e)

def free_func():
    super()

try:
    free_func()
except RuntimeError as e:
    print(e)

try:
    super(X, y).nonexistent_attr
except AttributeError as e:
    print(e)
