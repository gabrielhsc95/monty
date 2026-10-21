use std::fmt::Write;

use monty_types::MontyUuid;

use super::{Dict, LazyHeapSet, List, PyTrait, Type, attribute_name_value, tuple::allocate_tuple};
use crate::{
    args::ArgValues,
    boundary_uuid::create_uuid,
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, identity_hash},
    heap::{
        BorrowedHeapReadMut, DropGuard, DropWithContext, Heap, HeapId, HeapItem, HeapObjectRead, HeapReadMut,
        heap_read_ref_as_field_mut,
    },
    heap_data::HeapData,
    intern::Interns,
    types::{Union, str::allocate_string},
    value::{EitherStr, Value},
};

/// A user-defined class object created by a `class Foo: ...` statement.
///
/// Holds the class name and a `namespace` [`Dict`] mapping member names to values:
/// methods (stored as `DefFunction`/`Closure` values) and class variables. The
/// class's own [`HeapId`] is its type identity — `type(x) is Foo` and `isinstance`
/// work via reference identity, so there is no separate type-id counter.
///
/// Calling a class (`Foo(...)`) constructs an [`Instance`](super::Instance); see
/// `instantiate_class` in the VM's call module. Inheritance is not yet supported,
/// but a future `bases: Vec<HeapId>` field would slot in here without disturbing
/// the rest of the design.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Class {
    /// Class name (e.g. `Foo`), used for `repr` and `__name__`. Interned for
    /// compiled `class` statements; heap-owned for classes created at runtime
    /// via the 3-arg `type(name, bases, dict)` form, whose name cannot be
    /// interned because the intern table is frozen after prepare.
    name: EitherStr,
    /// Members: method name / class-variable name -> value.
    namespace: Dict,
    #[serde(default)]
    bases: Vec<HeapId>,
    #[serde(default)]
    mro: Vec<HeapId>,
    #[serde(default)]
    has_explicit_object_base: bool,
    #[serde(default)]
    base_exception: Option<ExcType>,
    #[serde(default)]
    direct_exception: Option<ExcType>,
    /// Boundary identity, generated lazily the first time the class (or one of
    /// its instances) crosses to the host; dumped with the heap so it stays
    /// stable across restores.
    uuid: Option<MontyUuid>,
}

impl Class {
    /// Returns the direct base classes in declaration order.
    pub(crate) fn bases(&self) -> &[HeapId] {
        &self.bases
    }

    /// Whether `object` was explicitly included in direct base classes.
    pub(crate) fn has_explicit_object_base(&self) -> bool {
        self.has_explicit_object_base
    }

    /// The base exception type if this class derives from `BaseException` / `Exception`.
    pub(crate) fn base_exception(&self) -> Option<ExcType> {
        self.base_exception
    }

    /// The direct exception base if this class directly derives from an `ExcType`.
    pub(crate) fn direct_exception(&self) -> Option<ExcType> {
        self.direct_exception
    }

    /// Returns the method resolution order (base classes in C3 linearization order).
    pub(crate) fn mro(&self) -> &[HeapId] {
        &self.mro
    }

    /// Whether this class inherits from `target`.
    pub(crate) fn is_subclass_of(&self, target: HeapId) -> bool {
        self.mro.contains(&target)
    }

    /// Searches the class namespace and base classes in C3 MRO order, returning a borrowed reference.
    pub(crate) fn mro_getattr_ref<'a>(
        &'a self,
        attr_str: &str,
        heap: &'a Heap,
        interns: &Interns,
    ) -> Option<&'a Value> {
        // 1. Check own namespace
        if let Some(value) = self.namespace.get_by_str(attr_str, heap, interns) {
            return Some(value);
        }

        // 2. Check base classes in C3 MRO order
        for &base_id in &self.mro {
            if let HeapData::Class(base_class) = heap.get(base_id)
                && let Some(val) = base_class.namespace.get_by_str(attr_str, heap, interns)
            {
                return Some(val);
            }
        }

        None
    }

    /// Searches the class namespace and base classes in C3 MRO order, cloning the result.
    pub(crate) fn mro_getattr(&self, attr_str: &str, heap: &Heap, interns: &Interns) -> Option<Value> {
        self.mro_getattr_ref(attr_str, heap, interns)
            .map(|v| v.clone_with_heap(heap))
    }

    /// Creates a new class object from its name, member namespace, direct bases, and C3 MRO.
    #[must_use]
    pub fn new(
        name: EitherStr,
        namespace: Dict,
        bases: Vec<HeapId>,
        mro: Vec<HeapId>,
        has_explicit_object_base: bool,
        base_exception: Option<ExcType>,
        direct_exception: Option<ExcType>,
    ) -> Self {
        Self {
            name,
            namespace,
            bases,
            mro,
            has_explicit_object_base,
            base_exception,
            direct_exception,
            uuid: None,
        }
    }

    /// Boundary identity of the class, generated and stored on first use so
    /// repeated crossings (and dump/restore) observe the same id. Only
    /// `Heap::boundary_uuid` may call this: it also indexes the new id.
    pub(crate) fn boundary_uuid(&mut self) -> MontyUuid {
        *self.uuid.get_or_insert_with(create_uuid)
    }

    /// The boundary identity, if the class (or an instance) has crossed to the host.
    #[must_use]
    pub fn uuid(&self) -> Option<MontyUuid> {
        self.uuid
    }

    /// Returns the class name (interned or heap-owned).
    #[must_use]
    pub fn name(&self) -> &EitherStr {
        &self.name
    }

    /// Returns a reference to the class member namespace.
    #[must_use]
    pub fn namespace(&self) -> &Dict {
        &self.namespace
    }
}

impl<'h> HeapReadMut<'h, Class> {
    /// Borrows the inline namespace for reader-gated attribute mutation.
    fn namespace_mut(&self) -> BorrowedHeapReadMut<'_, 'h, Dict> {
        heap_read_ref_as_field_mut!(self, Class, namespace)
    }

    /// Sets a class attribute (`Foo.x = 1`), returning the previous value (if any)
    /// for the caller to drop. Takes ownership of both `name` and `value`.
    ///
    /// Existing instances observe the change immediately: instance attribute reads
    /// fall through to this namespace.
    pub fn set_attr(&self, name: Value, value: Value, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        if let Some(name_str) = name.as_either_str(vm.heap)
            && matches!(name_str.as_str(vm.interns), "__class__" | "__bases__")
        {
            [name, value].drop_with(vm);
            return Err(ExcType::type_error("type controls __class__ and __bases__"));
        }
        self.namespace_mut().set(name, value, vm)
    }

    /// Removes a class attribute, returning it for the caller to drop.
    ///
    /// Only `@dataclass` needs this, to clear the `field()` object a class body
    /// bound for a factory-only field — CPython's decorator `delattr`s it.
    pub fn pop_attr(&self, name: &Value, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        Ok(self.namespace_mut().pop(name, vm)?.map(|(key, value)| {
            key.drop_with(vm);
            value
        }))
    }
}

fn class_mro_values(class_id: HeapId, mro: &[HeapId], base_exception: Option<ExcType>, heap: &mut Heap) -> Vec<Value> {
    let mut items = Vec::with_capacity(2 + mro.len() + 4);
    heap.inc_ref(class_id);
    items.push(Value::Ref(class_id));
    for &id in mro {
        heap.inc_ref(id);
        items.push(Value::Ref(id));
    }
    if let Some(exc) = base_exception {
        let mut cur = Some(exc);
        while let Some(e) = cur {
            items.push(Value::Builtin(Builtins::ExcType(e)));
            cur = e.parent();
        }
    }
    items.push(Value::Builtin(Builtins::Type(Type::Object)));
    items
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, Class> {
    /// Constructing an instance, which runs `__init__` as an ordinary frame.
    fn py_call(&mut self, args: ArgValues, vm: &mut VM<'h>) -> RunResult<CallResult> {
        vm.instantiate_class(self.id(), args)
    }

    fn py_type(&self, _vm: &VM<'h>) -> Type {
        // The type of a class object is `type` (matching `type(Foo) is type`).
        Type::Type
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_or_impl(&self, other: &Value, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        Union::heap_or(self, other, vm)
    }

    fn py_ror_impl(&self, other: &Value, vm: &mut VM<'h>) -> RunResult<Option<Value>> {
        Union::heap_ror(self, other, vm)
    }

    /// `Foo[int]`: a class's `__class_getitem__` is never looked up, so the
    /// wording is CPython's for a type without one.
    fn py_getitem(&self, _key: &Value, vm: &mut VM<'h>) -> RunResult<Value> {
        Err(ExcType::type_error_type_not_subscriptable(
            self.get(vm.heap).name.as_str(vm.interns),
        ))
    }

    fn py_set_attr(&mut self, name: &EitherStr, value: Value, vm: &mut VM<'h>) -> RunResult<()> {
        let mut value_guard = DropGuard::new(value, vm);
        let name = attribute_name_value(name, value_guard.ctx());
        let (value, vm) = value_guard.into_parts();
        let old_value = self.set_attr(name, value, vm)?;
        old_value.drop_with(vm);
        Ok(())
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        // Classes return `NotImplemented`; rich equality's final identity
        // fallback makes a class equal only to itself.
        Ok(None)
    }

    fn py_hash(&self, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        // Class objects hash by identity (like CPython type objects).
        Ok(Some(identity_hash(self.id())))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        Ok(write!(f, "<class '{}'>", self.get(vm.heap).name.as_str(vm.interns))?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        let attr_str = attr.as_str(vm.interns);
        // `Foo.__name__` returns the class name — before the namespace lookup
        // because in CPython `type.__name__` is a metaclass data descriptor that
        // shadows a same-named class-dict member (`class Foo: __name__ = 'bar'`
        // still reads `'Foo'`; only instances see the member).
        if attr_str == "__name__" {
            let name = self.get(vm.heap).name.as_str(vm.interns).to_owned();
            return Ok(Some(CallResult::Value(allocate_string(name, vm.heap))));
        }

        if attr_str == "__class__" {
            return Ok(Some(CallResult::Value(Value::Builtin(Builtins::Function(
                BuiltinsFunctions::Type,
            )))));
        }

        if attr_str == "__bases__" {
            let class = self.get(vm.heap);
            let bases = class.bases();
            let mut items = Vec::with_capacity(bases.len() + 1);
            for &base_id in bases {
                vm.heap.inc_ref(base_id);
                items.push(Value::Ref(base_id));
            }
            if let Some(exc) = class.direct_exception() {
                items.push(Value::Builtin(Builtins::ExcType(exc)));
            } else if bases.is_empty() || class.has_explicit_object_base() {
                items.push(Value::Builtin(Builtins::Type(Type::Object)));
            }
            let tuple_val = allocate_tuple(items.into(), vm.heap);
            return Ok(Some(CallResult::Value(tuple_val)));
        }

        if attr_str == "__mro__" {
            let class_id = self.id();
            let (mro, base_exception) = {
                let class = self.get(vm.heap);
                (class.mro().to_vec(), class.base_exception())
            };
            let items = class_mro_values(class_id, &mro, base_exception, vm.heap);
            let tuple_val = allocate_tuple(items.into(), vm.heap);
            return Ok(Some(CallResult::Value(tuple_val)));
        }

        // Look up a member in the class namespace or its base classes (MRO).
        if let Some(val) = self.get(vm.heap).mro_getattr(attr_str, vm.heap, vm.interns) {
            return Ok(Some(CallResult::Value(val)));
        }

        Err(ExcType::attribute_error_type(
            self.get(vm.heap).name.as_str(vm.interns),
            attr_str,
        ))
    }

    fn py_call_attr(&mut self, vm: &mut VM<'h>, attr: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        let attr_str = attr.as_str(vm.interns);
        // `__name__` is a synthesized string, not a namespace member (see
        // `py_getattr`), so calling it goes through the normal callable
        // dispatch and raises CPython's `TypeError: 'str' object is not
        // callable` rather than a spurious `AttributeError`.
        if attr_str == "__name__" {
            let name = self.get(vm.heap).name.as_str(vm.interns).to_owned();
            let name_val = allocate_string(name, vm.heap);
            defer_drop!(name_val, vm);
            return vm.call_function(name_val, args);
        }

        if attr_str == "mro" {
            let class_name = self.get(vm.heap).name.as_str(vm.interns).to_owned();
            let pos = args.into_pos_only(&format!("{class_name}.mro"), vm.heap)?;
            if !pos.as_slice().is_empty() {
                let count = pos.as_slice().len();
                pos.drop_with(vm.heap);
                return Err(ExcType::type_error(format!(
                    "{class_name}.mro() takes no arguments ({count} given)"
                )));
            }
            let class_id = self.id();
            let (mro, base_exception) = {
                let class = self.get(vm.heap);
                (class.mro().to_vec(), class.base_exception())
            };
            let items = class_mro_values(class_id, &mro, base_exception, vm.heap);
            let list_id = vm.heap.allocate(HeapData::List(List::new(items)));
            return Ok(CallResult::Value(Value::Ref(list_id)));
        }

        // `Foo.method(args)` calls the raw (unbound) member with the given args —
        // no `self` is inserted, the caller passes the instance explicitly.
        if let Some(member) = self.get(vm.heap).mro_getattr(attr_str, vm.heap, vm.interns) {
            defer_drop!(member, vm);
            return vm.call_function(member, args);
        }

        if attr_str == "__init_subclass__" {
            let (pos, kwargs) = args.into_parts();
            let pos_len = pos.as_slice().len();
            pos.drop_with(vm.heap);
            let has_kwargs = !kwargs.is_empty();
            kwargs.drop_with(vm.heap);
            let class_name = self.get(vm.heap).name.as_str(vm.interns);
            if pos_len > 0 {
                return Err(ExcType::type_error(format!(
                    "{class_name}.__init_subclass__() takes no arguments ({pos_len} given)"
                )));
            }
            if has_kwargs {
                return Err(ExcType::type_error(format!(
                    "{class_name}.__init_subclass__() takes no keyword arguments"
                )));
            }
            return Ok(CallResult::Value(Value::None));
        }

        args.drop_with(vm);
        Err(ExcType::attribute_error_type(
            self.get(vm.heap).name.as_str(vm.interns),
            attr_str,
        ))
    }
}

impl HeapItem for Class {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.namespace.py_dec_ref_ids(stack);
        for &base_id in &self.bases {
            stack.push(base_id);
        }
    }
}
