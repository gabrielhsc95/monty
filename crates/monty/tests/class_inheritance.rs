use monty::{Dump, MontyRepl, MontyRun, Session, SessionRef, dump};
use monty_types::{CompileOptions, MontyObject, PrintWriter, ResourceLimits, ResourceTracker};

fn run(code: &str) {
    MontyRun::new(code.to_owned(), "test.py", vec![], CompileOptions::default())
        .unwrap()
        .run_no_limits(vec![])
        .unwrap();
}

#[test]
fn parent_evaluates_once_before_class_body() {
    run(r"
events = []

class Parent:
    pass

def parent():
    events.append('parent')
    return Parent

def member():
    events.append('body')
    return 1

class Child(parent()):
    value = member()

assert events == ['parent', 'body']
assert Child.value == 1
");
}

#[test]
fn parent_failure_prevents_class_body_execution() {
    run(r"
events = []

def member():
    events.append('body')
    return 1

try:
    class Child(MissingParent):
        value = member()
except NameError:
    pass
else:
    assert False, 'an undefined parent must raise NameError'

assert events == []
");
}

#[test]
fn parent_argument_and_class_body_capture_use_distinct_slots() {
    run(r"
class Parent:
    pass

def outer(Parent):
    events = []
    def inner():
        class Child(Parent):
            Parent = events.append('body')
    inner()
    return events

assert outer(Parent) == ['body']
");
}

#[test]
fn single_parent_classes_can_be_constructed() {
    run(r"
class Parent:
    pass

class Child(Parent):
    value = 1

class Grandchild(Child):
    value = 2

assert Child.value == 1
assert Grandchild.value == 2
assert type(Child()) is Child
assert type(Grandchild()) is Grandchild

Dynamic = type('Dynamic', (Parent,), {'value': 3})
assert Dynamic.value == Dynamic().value == 3
assert type(Dynamic()) is Dynamic
");
}

#[test]
fn unsupported_bases_remain_rejected() {
    run(r"
class Parent:
    pass

for base in [42, int, Parent()]:
    try:
        type('Child', (base,), {})
    except TypeError:
        pass
    else:
        assert False, 'only sandbox-defined classes can be parents'

try:
    type('Child', (Parent, Parent), {})
except TypeError as exc:
    assert 'duplicate base class' in str(exc)
else:
    assert False, 'duplicate base class must be rejected'
");
}

#[test]
fn inherited_member_lookup_matches_examples() {
    run(include_str!("../test_cases/class__inheritance_basic.py"));
    run(include_str!("../test_cases/class__inheritance_lookup.py"));
    run(include_str!("../test_cases/class__inheritance_mutation.py"));
}

#[test]
fn inherited_instance_checks_match_examples() {
    run(include_str!("../test_cases/class__inheritance_isinstance.py"));
    run(include_str!("../test_cases/class__inheritance_dynamic.py"));
    run(include_str!("../test_cases/class__inheritance_redefinition.py"));
}

#[test]
fn inherited_special_methods_match_examples() {
    run(include_str!("../test_cases/class__inheritance_dunders.py"));
}

#[test]
fn subclass_checks_match_examples() {
    run(include_str!("../test_cases/class__inheritance_type_checks.py"));
}

#[test]
fn deeply_nested_subclass_checks_respect_recursion_limits() {
    let code = r"
class Parent: pass
class Child(Parent): pass
nested = Parent
for _ in range(100):
    nested = (nested,)
assert issubclass(Child, (Parent, nested))
try:
    issubclass(Child, nested)
except RecursionError:
    pass
else:
    assert False, 'nested classinfo must respect the recursion limit'
assert issubclass(Child, Parent)
";
    let tracker = ResourceTracker::new(ResourceLimits {
        max_recursion_depth: 16,
        ..ResourceLimits::default()
    });
    MontyRun::new(code.to_owned(), "test.py", vec![], CompileOptions::default())
        .unwrap()
        .run(vec![], tracker, PrintWriter::Stdout)
        .unwrap();
}

#[test]
fn inherited_initialization_handles_defaults_overrides_and_mutation() {
    run(r"
class Parent:
    def __init__(self, value, *, offset=2):
        self.value = value + offset

class Child(Parent):
    pass

class Grandchild(Child):
    pass

assert Child(3).value == 5
assert Grandchild(3, offset=4).value == 7
Dynamic = type('Dynamic', (Grandchild,), {})
assert Dynamic(3).value == 5
assert type(Dynamic(3)) is Dynamic

class OwnInit(Parent):
    def __init__(self):
        self.value = 99

assert OwnInit().value == 99

def replacement(self, value=10):
    self.value = value * 2

Parent.__init__ = replacement
assert Child().value == 20
assert Grandchild(4).value == 8
assert OwnInit().value == 99
");
}

#[test]
fn inherited_initializer_errors_preserve_constructor_contracts() {
    run(r#"
class Parent:
    def __init__(self):
        raise ValueError('from parent')

class Child(Parent):
    pass

try:
    Child()
except ValueError as exc:
    assert str(exc) == 'from parent'
else:
    assert False, 'inherited initialization must propagate exceptions'

class Blocked(Parent):
    __init__ = None

try:
    Blocked()
except TypeError:
    pass
else:
    assert False, 'None must shadow the parent initializer'

def bad_return(self):
    return 42

Parent.__init__ = bad_return
try:
    Child()
except TypeError as exc:
    assert str(exc) == "__init__() should return None, not 'int'"
else:
    assert False, 'inherited initialization must return None'

class Empty:
    pass

class EmptyChild(Empty):
    pass

assert type(EmptyChild()) is EmptyChild
try:
    EmptyChild(1)
except TypeError:
    pass
else:
    assert False, 'classes without an initializer reject arguments'
"#);
}

#[test]
fn inherited_lookup_survives_dump_restore() {
    let mut repl = MontyRepl::new("test.py", ResourceTracker::default(), CompileOptions::default());
    repl.feed_run(
        "class Parent:\n    value = 7\nclass Child(Parent): pass\ninstance = Child()\nSavedParent = Parent\nParent = None\ncheck_subclass = issubclass",
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();
    let bytes = dump("test.py", None, &[], SessionRef::Idle(&repl)).unwrap();
    let Session::Idle(mut restored) = Dump::load(&bytes).unwrap().state else {
        panic!("expected an idle session");
    };
    let output = restored
        .feed_run(
            "assert isinstance(instance, SavedParent)\nassert check_subclass(Child, SavedParent)\ninstance.value + Child.value",
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
    assert_eq!(output, MontyObject::int(14));
}

fn resume_inherited_initializer(round_trip: bool) {
    let repl = MontyRepl::new("test.py", ResourceTracker::default(), CompileOptions::default());
    let progress = repl
        .feed_start(
            "class Parent:\n    def __init__(self):\n        self.value = ext_fn()\nclass Child(Parent): pass\ninstance = Child()\nassert type(instance) is Child\ninstance.value",
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
    let progress = if round_trip {
        let bytes = dump("test.py", None, &[], SessionRef::Suspended(&progress)).unwrap();
        let Session::Suspended(restored) = Dump::load(&bytes).unwrap().state else {
            panic!("expected a suspended initializer");
        };
        *restored
    } else {
        progress
    };
    let call = progress.into_function_call().expect("expected an external call");
    assert_eq!(call.function_name, "ext_fn");
    let completed = call.resume(MontyObject::int(41), PrintWriter::Stdout).unwrap();
    let (_, output) = completed.into_complete().expect("expected construction to complete");
    assert_eq!(output, MontyObject::int(41));
}

#[test]
fn inherited_initializer_can_suspend() {
    resume_inherited_initializer(false);
}

#[test]
#[cfg(not(feature = "memory-model-checks"))]
fn inherited_initializer_resumes_after_dump_restore() {
    resume_inherited_initializer(true);
}

#[test]
#[cfg(feature = "ref-count-return")]
fn parent_child_cycles_are_collected() {
    let code = r"
def make_cycle():
    class Parent:
        pass
    class Child(Parent):
        pass
    Parent.child = Child

for _ in range(10):
    make_cycle()
";
    let output = MontyRun::new(code.to_owned(), "test.py", vec![], CompileOptions::default())
        .unwrap()
        .run_ref_counts(vec![])
        .unwrap();
    assert!(output.unreachable.is_empty(), "{:?}", output.unreachable);
}

#[test]
#[cfg(feature = "ref-count-return")]
fn rejected_class_namespaces_release_parent_references() {
    let code = r"
def exercise():
    class Parent: pass
    for namespace in [None, {1: 'invalid key'}]:
        try:
            type('Child', (Parent,), namespace)
        except TypeError:
            pass
for _ in range(10):
    exercise()
";
    let output = MontyRun::new(code.to_owned(), "test.py", vec![], CompileOptions::default())
        .unwrap()
        .run_ref_counts(vec![])
        .unwrap();
    assert!(output.unreachable.is_empty(), "{:?}", output.unreachable);
}

#[test]
fn custom_new_controls_allocation_and_initialization() {
    run(r"
events = []
class Parent:
    def __new__(cls, value):
        events.append('new')
        result = object.__new__(cls)
        result.from_new = value
        return result
    def __init__(self, value):
        events.append('init')
        self.value = value

class Child(Parent):
    pass

child = Child(7)
assert type(child) is Child
assert child.from_new == child.value == 7
assert events == ['new', 'init']
assert child.__new__(Child, 8).__class__ is Child

class ReturnsNumber:
    def __new__(cls):
        return 42
    def __init__(self):
        assert False

assert ReturnsNumber() == 42

class NewOnly:
    def __new__(cls, value):
        result = object.__new__(cls)
        result.value = value
        return result

assert NewOnly(5).value == 5

class Other:
    pass

class ReturnsOther:
    def __new__(cls):
        return Other()
    def __init__(self):
        assert False

assert type(ReturnsOther()) is Other
");
}

#[test]
fn classes_and_bases_cannot_be_reassigned() {
    run(r"
class Parent:
    pass

class Child(Parent):
    pass

obj = Child()
for target in [Parent, Child, obj]:
    for name in ['__class__', '__bases__']:
        try:
            setattr(target, name, Parent)
        except TypeError:
            pass
        else:
            assert False

try:
    object.__setattr__(obj, '__class__', Parent)
except TypeError:
    pass
else:
    assert False

assert obj.__class__ is Child
assert Parent.__bases__ == (object,)
assert Child.__class__ is type
assert Child.__bases__ == (Parent,)
assert isinstance(obj, Parent)
");
}

#[test]
fn native_exception_ancestry_and_identity_are_preserved() {
    run(r#"
class Error(ValueError):
    pass

class Child(Error):
    pass

error = Child('message', [1, 2])
assert type(error) is Child
assert error.args == ('message', [1, 2])
assert isinstance(error, ValueError)
assert isinstance(error, Exception)
assert isinstance(error, Error)
assert issubclass(Child, ValueError)
assert Child.__bases__ == (Error,)
assert Error.__bases__ == (ValueError,)
try:
    raise error
except Error as caught:
    assert caught is error
    try:
        raise
    except ValueError as reraised:
        assert reraised is error
else:
    assert False

try:
    raise Child
except Exception as caught:
    assert type(caught) is Child
else:
    assert False

class Custom(Exception):
    def __new__(cls, message):
        return Exception.__new__(cls, message)
    def __init__(self, message):
        self.message = message

custom = Custom('hello')
assert custom.args == ('hello',)
assert str(custom) == 'hello'
assert repr(custom) == "Custom('hello')"
try:
    raise custom
except Custom as caught:
    assert caught is custom
else:
    assert False

class Context:
    def __enter__(self):
        return self
    def __exit__(self, typ, value, traceback):
        assert typ is Child
        assert value is error
        return True

with Context():
    raise error

Dynamic = type('Dynamic', (Error,), {})
assert isinstance(Dynamic(), ValueError)
"#);
}

#[test]
fn object_is_the_root_of_custom_and_native_classes() {
    run(r"
class Parent:
    pass

class Child(Parent):
    pass

class Error(ValueError):
    pass

assert object.__bases__ == ()
assert Parent.__bases__ == (object,)
assert Child.__bases__ == (Parent,)
assert ValueError.__bases__ == (Exception,)
assert Exception.__bases__ == (BaseException,)
assert BaseException.__bases__ == (object,)
assert bool.__bases__ == (int,)
assert type.__bases__ == (object,)
import collections
Point = collections.namedtuple('Point', ['x'])
assert Point.__bases__ == (tuple,)
assert Point.__class__ is type
assert issubclass(Point, object)
assert isinstance(Point(1), object)

for cls in (Parent, Child, Error, ValueError, Exception, BaseException, bool, int, dict, tuple, str, type):
    assert issubclass(cls, object)
    assert cls.__class__ is type
for value in (Parent(), Child(), [], Error('x'), ValueError('x'), True, 1, {}, (), 'x', object):
    assert isinstance(value, object)
");
}

#[test]
fn super_zero_arg_and_cooperative_dispatch() {
    run(r#"
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
        return f"{super().speak()} and barks"

d = Dog("Rex", "Collie")
assert d.name == "Rex"
assert d.breed == "Collie"
assert d.speak() == "Rex makes a sound and barks"

# Diamond multiple inheritance cooperative dispatch
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
assert d_obj.greet() == ["D", "B", "C", "A"]
"#);
}

#[test]
fn super_two_arg_and_introspection() {
    run(r#"
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
assert s.foo() == "Sub.foo"
assert s.mid_foo() == "Mid.foo"
assert s.base_foo() == "Base.foo"

# super(Type, SubType)
class Shape:
    kind = "generic shape"

class Polygon(Shape):
    kind = "polygon"

class Triangle(Polygon):
    kind = "triangle"

assert super(Triangle, Triangle).kind == "polygon"
assert super(Polygon, Triangle).kind == "generic shape"

# Introspection attributes
class X:
    pass

class Y(X):
    def check_super(self):
        s = super()
        assert s.__thisclass__ is Y
        assert s.__self__ is self
        assert s.__self_class__ is Y

y = Y()
y.check_super()

s_unbound = super(Y)
assert repr(s_unbound) == "<super: <class 'Y'>, NULL>"
assert s_unbound.__thisclass__ is Y
assert s_unbound.__self__ is None
assert s_unbound.__self_class__ is None

try:
    s_unbound.some_attr
except AttributeError as e:
    assert "'super' object has no attribute 'some_attr'" in str(e)

s_bound = super(Y, y)
assert repr(s_bound) == "<super: <class 'Y'>, <Y object>>"

s_class_bound = super(X, Y)
assert repr(s_class_bound) == "<super: <class 'X'>, <Y object>>"

# object.__init__ via super()
class Leaf:
    def __init__(self):
        super().__init__()

Leaf()
"#);
}

#[test]
fn super_errors() {
    run(r#"
class X:
    pass

class Y(X):
    pass

# Arg 1 not a type
try:
    super(1, 2)
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "super() argument 1 must be a type, not int"

# Arg 2 not an instance or subtype
try:
    super(X, "not_an_instance")
    assert False, "expected TypeError"
except TypeError as e:
    assert "is not an instance or subtype of type" in str(e)

try:
    super(Y, X())
    assert False, "expected TypeError"
except TypeError as e:
    assert "is not an instance or subtype of type" in str(e)

# Kwargs rejected
try:
    super(foo=1)
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "super() takes no keyword arguments"

# Too many arguments
try:
    super(X, Y(), 1)
    assert False, "expected TypeError"
except TypeError as e:
    assert "super() expected at most 2 arguments, got 3" in str(e)

# Free function zero-arg super
def free_func():
    super()

try:
    free_func()
    assert False, "expected RuntimeError"
except RuntimeError as e:
    assert "super(): no arguments" in str(e)

# Zero-arg super outside any frame
try:
    eval("super()")
    assert False, "expected RuntimeError"
except RuntimeError as e:
    assert "super(): no current frame" in str(e) or "super(): no arguments" in str(e)
"#);
}

#[test]
fn init_subclass_hooks_and_class_kwargs() {
    run(r#"
class PluginBase:
    registry = []
    def __init_subclass__(cls, plugin_name="default", **kwargs):
        super().__init_subclass__(**kwargs)
        cls.plugin_name = plugin_name
        cls.registry.append(cls)

class PluginA(PluginBase, plugin_name="alpha"):
    pass

class PluginB(PluginBase, plugin_name="beta"):
    pass

assert PluginA.plugin_name == "alpha"
assert PluginB.plugin_name == "beta"
assert PluginBase.registry == [PluginA, PluginB]

# Base definition does not call its own __init_subclass__
calls = []
class Tracker:
    def __init_subclass__(cls, **kwargs):
        super().__init_subclass__(**kwargs)
        calls.append(cls.__name__)

assert calls == []

class SubTracker(Tracker):
    pass

assert calls == ["SubTracker"]

# Cooperative multi-level kwargs forwarding
log = []
class A:
    def __init_subclass__(cls, a=1, **kwargs):
        super().__init_subclass__(**kwargs)
        log.append(("A", cls.__name__, a))

class B(A):
    def __init_subclass__(cls, b=2, **kwargs):
        super().__init_subclass__(**kwargs)
        log.append(("B", cls.__name__, b))

assert log == [("A", "B", 1)]

class C(B, a=10, b=20):
    pass

assert log == [("A", "B", 1), ("A", "C", 10), ("B", "C", 20)]

# Dynamic type() call with kwargs
class DynBase:
    def __init_subclass__(cls, tag="", **kwargs):
        super().__init_subclass__(**kwargs)
        cls.tag = tag

DynSub = type("DynSub", (DynBase,), {}, tag="dynamic_plugin")
assert DynSub.tag == "dynamic_plugin"

# Direct __init_subclass__ call on builtin
assert int.__init_subclass__() is None
"#);
}

#[test]
fn init_subclass_and_kwargs_errors() {
    run(r#"
# Keyword arguments without __init_subclass__
try:
    class Standalone(extra="val"):
        pass
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "Standalone.__init_subclass__() takes no keyword arguments"

try:
    class ObjSub(object, extra="val"):
        pass
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "ObjSub.__init_subclass__() takes no keyword arguments"

# Direct invocation argument validation
class Foo:
    pass

try:
    Foo.__init_subclass__(1)
    assert False, "expected TypeError"
except TypeError as e:
    assert "takes no arguments (1 given)" in str(e)

try:
    Foo.__init_subclass__(extra=1)
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "Foo.__init_subclass__() takes no keyword arguments"

try:
    int.__init_subclass__(1)
    assert False, "expected TypeError"
except TypeError as e:
    assert "takes no arguments (1 given)" in str(e)

try:
    int.__init_subclass__(extra=1)
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "int.__init_subclass__() takes no keyword arguments"

# Failing __init_subclass__ aborts class creation
class FailingBase:
    def __init_subclass__(cls, **kwargs):
        raise ValueError("initialization failed")

try:
    class Child(FailingBase):
        pass
    assert False, "expected ValueError"
except ValueError as e:
    assert str(e) == "initialization failed"
"#);
}

#[test]
fn issubclass_builtin_and_type_checks() {
    run(r"
class Grandparent:
    pass

class Parent(Grandparent):
    pass

class Child(Parent):
    pass

class Unrelated:
    pass

assert issubclass(Child, Parent)
assert issubclass(Child, Grandparent)
assert issubclass(Child, object)
assert issubclass(Child, Child)
assert not issubclass(Parent, Child)
assert not issubclass(Unrelated, Parent)

# Builtin types
assert issubclass(bool, int)
assert issubclass(bool, object)
assert issubclass(int, object)
assert issubclass(str, object)
assert not issubclass(int, str)
assert not issubclass(str, int)

# Tuples as second arg
assert issubclass(Child, (int, Parent))
assert issubclass(Child, (Unrelated, (str, (float, Grandparent))))
assert not issubclass(Child, (int, str, Unrelated))

# Unions
assert issubclass(int, int | str)
assert issubclass(bool, str | int)
assert not issubclass(str, int | float)

# Type object itself
assert issubclass(type, object)
assert issubclass(type, type)
assert issubclass(Child, object)
assert not issubclass(Child, type)
");
}

#[test]
fn issubclass_errors() {
    run(r#"
class Base:
    pass

# Arg 1 not a class
try:
    issubclass(1, int)
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "issubclass() arg 1 must be a class"

try:
    issubclass(Base(), Base)
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "issubclass() arg 1 must be a class"

# Arg 2 not a class, tuple, or union
try:
    issubclass(Base, 1)
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "issubclass() arg 2 must be a class, a tuple of classes, or a union"

try:
    issubclass(int, (str, 1))
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "issubclass() arg 2 must be a class, a tuple of classes, or a union"

# Generics rejected
try:
    issubclass(int, list[int])
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "issubclass() argument 2 cannot be a parameterized generic"

try:
    issubclass(list[int], list)
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "issubclass() arg 1 must be a class"
"#);
}

#[test]
fn class_introspection_mro_and_bases() {
    run(r#"
class A:
    pass

class B(A):
    pass

class C(A):
    pass

class D(B, C):
    pass

assert A.__bases__ == (object,)
assert B.__bases__ == (A,)
assert D.__bases__ == (B, C)
assert object.__bases__ == ()
assert bool.__bases__ == (int,)
assert int.__bases__ == (object,)

assert D.__mro__ == (D, B, C, A, object)
assert D.mro() == [D, B, C, A, object]
assert bool.mro() == [bool, int, object]
assert object.mro() == [object]
assert int.mro() == [int, object]
assert type.mro() == [type, object]

# Calling mro with arguments raises TypeError
try:
    D.mro(1)
    assert False, "expected TypeError"
except TypeError as e:
    assert "D.mro() takes no arguments (1 given)" in str(e)

try:
    bool.mro(1)
    assert False, "expected TypeError"
except TypeError as e:
    assert "bool.mro() takes no arguments (1 given)" in str(e)

try:
    type.mro(1)
    assert False, "expected TypeError"
except TypeError as e:
    assert "type.mro() takes no arguments (1 given)" in str(e)

# Reassignment protection
try:
    D.__bases__ = ()
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "type controls __class__ and __bases__"

try:
    D.__class__ = object
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "type controls __class__ and __bases__"

# __eq__ without __hash__ sets __hash__ to None
class EqOnly:
    def __eq__(self, other):
        return True

assert EqOnly.__hash__ is None
try:
    hash(EqOnly())
    assert False, "expected TypeError"
except TypeError as e:
    assert "unhashable type: 'EqOnly'" in str(e)
"#);
}

#[test]
fn custom_exception_subclasses_and_matching() {
    run(r#"
class AppError(Exception):
    pass

class DatabaseError(AppError):
    def __init__(self, code, message):
        super().__init__()
        self.code = code
        self.message = message

class QueryError(DatabaseError):
    pass

assert issubclass(QueryError, DatabaseError)
assert issubclass(QueryError, AppError)
assert issubclass(QueryError, Exception)
assert issubclass(QueryError, BaseException)
assert QueryError.__mro__ == (QueryError, DatabaseError, AppError, Exception, BaseException, object)

# Raise and catch
caught = None
try:
    raise QueryError(1001, "table not found")
except AppError as e:
    caught = e
    assert isinstance(e, QueryError)
    assert isinstance(e, DatabaseError)
    assert isinstance(e, AppError)
    assert isinstance(e, Exception)
    assert e.code == 1001
    assert e.message == "table not found"

assert caught is not None

# ExcType.mro() and custom exception .mro()
assert ValueError.mro() == [ValueError, Exception, BaseException, object]
assert QueryError.mro() == [QueryError, DatabaseError, AppError, Exception, BaseException, object]

try:
    ValueError.mro(1)
    assert False, "expected TypeError"
except TypeError as e:
    assert "ValueError.mro() takes no arguments (1 given)" in str(e)

# Exception layout conflicts
try:
    class DuplicateErr(ValueError, ValueError):
        pass
    assert False, "expected TypeError"
except TypeError as e:
    assert "duplicate base class ValueError" in str(e)

try:
    class ConflictingErr(ValueError, TypeError):
        pass
    assert False, "expected TypeError"
except TypeError as e:
    assert "multiple bases have instance lay-out conflict" in str(e)
"#);
}

#[test]
fn object_new_builtin_and_custom_new() {
    run(r#"
class Plain:
    def __init__(self):
        self.x = 10

p = object.__new__(Plain)
assert isinstance(p, Plain)
assert not hasattr(p, "x")

# Calling object.__new__ on native type
try:
    object.__new__(int)
    assert False, "expected TypeError"
except TypeError as e:
    assert "object.__new__(X): X is not a type object" in str(e)

# Calling object.__new__ on non-type
try:
    object.__new__(123)
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "object.__new__(X): X is not a type object (int)"

# Calling object.__new__ without args
try:
    object.__new__()
    assert False, "expected TypeError"
except TypeError as e:
    assert str(e) == "object.__new__(): not enough arguments"

# User class custom __new__ returning instance vs non-instance
class Singleton:
    _instance = None
    def __new__(cls, val):
        if cls._instance is None:
            cls._instance = object.__new__(cls)
            cls._instance.val = val
        return cls._instance

s1 = Singleton(1)
s2 = Singleton(2)
assert s1 is s2
assert s1.val == 1

class Factory:
    def __new__(cls):
        return 42

assert Factory() == 42
"#);
}

#[test]
fn inheritance_copy_module_and_snapshots() {
    run(r#"
import copy

class Base:
    def __init__(self, x):
        self.x = x

class Sub(Base):
    def __init__(self, x, y):
        super().__init__(x)
        self.y = y

s = Sub([1, 2], {"a": 1})
s_shallow = copy.copy(s)
assert s_shallow is not s
assert s_shallow.x is s.x
assert s_shallow.y is s.y

s_deep = copy.deepcopy(s)
assert s_deep is not s
assert s_deep.x is not s.x
assert s_deep.x == [1, 2]
assert s_deep.y is not s.y
assert s_deep.y == {"a": 1}
"#);

    // Repl session snapshotting and restoring class hierarchy with super()
    let mut repl = MontyRepl::new("test.py", ResourceTracker::default(), CompileOptions::default());
    repl.feed_run(
        r"
class A:
    def val(self):
        return 10

class B(A):
    def val(self):
        return super().val() + 20

b = B()
",
        vec![],
        PrintWriter::Stdout,
    )
    .unwrap();

    let bytes = dump("test.py", None, &[], SessionRef::Idle(&repl)).unwrap();
    let Session::Idle(mut restored) = Dump::load(&bytes).unwrap().state else {
        panic!("expected an idle session");
    };

    let output = restored.feed_run("b.val()", vec![], PrintWriter::Stdout).unwrap();
    assert_eq!(output, MontyObject::int(30));
}

#[test]
fn inheritance_edge_cases_and_error_paths() {
    run(r#"
# isinstance with type
class A: pass
assert isinstance(A, type)
assert isinstance(int, type)
assert isinstance(ValueError, type)
assert isinstance(type, type)
assert not isinstance(1, type)
assert not isinstance(A(), type)

# isinstance with Exception types
class CustomErr(ValueError): pass
class OtherErr(TypeError): pass
err = CustomErr("msg")
assert isinstance(err, CustomErr)
assert isinstance(err, ValueError)
assert isinstance(err, Exception)
assert isinstance(err, BaseException)
assert not isinstance(err, TypeError)
assert not isinstance(123, Exception)

# Exception hierarchy MROs exercising ExcType::parent()
assert KeyError.mro() == [KeyError, LookupError, Exception, BaseException, object]
assert IndexError.mro() == [IndexError, LookupError, Exception, BaseException, object]
assert FileNotFoundError.mro() == [FileNotFoundError, OSError, Exception, BaseException, object]
assert ZeroDivisionError.mro() == [ZeroDivisionError, ArithmeticError, Exception, BaseException, object]
assert KeyboardInterrupt.mro() == [KeyboardInterrupt, BaseException, object]
assert SystemExit.mro() == [SystemExit, BaseException, object]

# Custom exception inheriting from BaseException directly
class CustomBaseExc(BaseException): pass
class SubCustomBaseExc(CustomBaseExc): pass
assert SubCustomBaseExc.__mro__ == (SubCustomBaseExc, CustomBaseExc, BaseException, object)
assert issubclass(SubCustomBaseExc, BaseException)
assert not issubclass(SubCustomBaseExc, Exception)

# Multiple bases conflict and duplicate bases
try:
    class DupParent(A, A): pass
    assert False
except TypeError as e:
    assert "duplicate base class A" in str(e)

try:
    class ConflictExc(ValueError, TypeError): pass
    assert False
except TypeError as e:
    assert "multiple bases have instance lay-out conflict" in str(e)

# Decorators combined with class kwargs
def tag(cls):
    cls.tagged = True
    return cls

class BasePlugin:
    def __init_subclass__(cls, k="default", **kwargs):
        super().__init_subclass__(**kwargs)
        cls.k = k

@tag
class ConcretePlugin(BasePlugin, k="custom"):
    pass

assert ConcretePlugin.tagged is True
assert ConcretePlugin.k == "custom"

# Unpack in bases with kwargs
bases = (BasePlugin,)
class UnpackSub(*bases, k="unpacked"):
    pass

assert UnpackSub.k == "unpacked"
"#);

    // Parse errors: metaclass and **kwargs unpacking
    assert!(
        MontyRun::new(
            "class WithMeta(metaclass=type): pass".to_owned(),
            "test.py",
            vec![],
            CompileOptions::default()
        )
        .is_err()
    );

    assert!(
        MontyRun::new(
            "kwargs = {}\nclass WithKwargsUnpack(**kwargs): pass".to_owned(),
            "test.py",
            vec![],
            CompileOptions::default()
        )
        .is_err()
    );
}
