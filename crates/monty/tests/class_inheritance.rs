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
