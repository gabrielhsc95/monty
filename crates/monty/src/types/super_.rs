//! Implementation of the super() builtin and the Super proxy type.

use std::{borrow::Cow, fmt::Write};

use super::{
    LazyHeapSet, PyTrait, Type,
    instance::{BoundMethod, class_name, is_method_value},
};
use crate::{
    args::ArgValues,
    builtins::{Builtins, isinstance::isinstance_check, issubclass::issubclass_check},
    bytecode::{CallResult, VM},
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, identity_hash},
    heap::{DropWithContext, HeapData, HeapId, HeapItem, HeapObjectRead},
    intern::FunctionId,
    value::{EitherStr, Value},
};

/// A `super` proxy object created by `super(...)`.
///
/// Implements cooperative multiple inheritance method resolution order (MRO)
/// dispatch. Holds the class to search after (`type_arg`) and the target
/// instance or subclass (`obj_arg`).
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Super {
    pub(crate) type_arg: Value,
    pub(crate) obj_arg: Option<Value>,
}

impl Super {
    #[must_use]
    pub fn new(type_arg: Value, obj_arg: Option<Value>) -> Self {
        Self { type_arg, obj_arg }
    }
}

/// Implementation of the `super(...)` builtin function.
pub fn builtin_super(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (pos_args, kwargs) = args.into_parts();
    if !kwargs.is_empty() {
        pos_args.drop_with(vm.heap);
        kwargs.drop_with(vm.heap);
        return Err(ExcType::type_error("super() takes no keyword arguments"));
    }
    match pos_args.len() {
        0 => zero_arg_super(vm),
        1 => {
            let mut pos = pos_args;
            let type_arg = pos.next().expect("length checked");
            if let Err(e) = validate_super_type(&type_arg, vm) {
                type_arg.drop_with(vm.heap);
                return Err(e);
            }
            let s = Super::new(type_arg, None);
            Ok(Value::Ref(vm.heap.allocate(HeapData::Super(s))))
        }
        2 => {
            let mut pos = pos_args;
            let type_arg = pos.next().expect("length checked");
            let obj_arg = pos.next().expect("length checked");
            if let Err(e) = validate_super_type(&type_arg, vm) {
                type_arg.drop_with(vm.heap);
                obj_arg.drop_with(vm.heap);
                return Err(e);
            }
            if let Err(e) = validate_super_obj(&type_arg, &obj_arg, vm) {
                type_arg.drop_with(vm.heap);
                obj_arg.drop_with(vm.heap);
                return Err(e);
            }
            let s = Super::new(type_arg, Some(obj_arg));
            Ok(Value::Ref(vm.heap.allocate(HeapData::Super(s))))
        }
        n => {
            pos_args.drop_with(vm.heap);
            Err(ExcType::type_error(format!(
                "super() expected at most 2 arguments, got {n}"
            )))
        }
    }
}

fn zero_arg_super(vm: &mut VM<'_>) -> RunResult<Value> {
    let (locals_count, base, func_id) = {
        let frame = vm.current_frame();
        (frame.locals_count(), frame.stack_base(), frame.function_id())
    };
    if locals_count == 0 {
        return Err(ExcType::runtime_error("super(): no arguments"));
    }
    let self_val = vm.stack[base].clone_with_heap(vm.heap);
    if matches!(self_val, Value::Undefined) {
        return Err(ExcType::runtime_error("super(): no arguments"));
    }
    defer_drop!(self_val, vm);

    let mut class_val = None;
    for slot in 0..locals_count {
        let name_match = vm
            .current_frame()
            .code()
            .local_name(slot)
            .is_some_and(|name_id| vm.interns.get_str(name_id) == "__class__");
        if name_match {
            let val = match vm.stack[base + usize::from(slot)] {
                Value::Ref(cell_id) => match vm.heap.get(cell_id) {
                    HeapData::Cell(cell) => cell.0.clone_with_heap(vm.heap),
                    _ => Value::Undefined,
                },
                ref other => other.clone_with_heap(vm.heap),
            };
            if !matches!(val, Value::Undefined) {
                class_val = Some(val);
                break;
            }
        }
    }

    if class_val.is_none()
        && let Some(fid) = func_id
    {
        class_val = find_defining_class(fid, self_val, vm);
    }

    let Some(class_val) = class_val else {
        return Err(ExcType::runtime_error("super(): __class__ cell not found"));
    };
    defer_drop!(class_val, vm);

    let type_arg = class_val.clone_with_heap(vm.heap);
    let obj_arg = self_val.clone_with_heap(vm.heap);
    let s = Super::new(type_arg, Some(obj_arg));
    Ok(Value::Ref(vm.heap.allocate(HeapData::Super(s))))
}

fn find_defining_class(func_id: FunctionId, self_val: &Value, vm: &VM<'_>) -> Option<Value> {
    let start_class_id = match self_val {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Instance(inst) => inst.class(),
            HeapData::Class(_) => *id,
            _ => return None,
        },
        _ => return None,
    };

    let HeapData::Class(start_class) = vm.heap.get(start_class_id) else {
        return None;
    };

    let mut mro_classes = Vec::with_capacity(1 + start_class.mro().len());
    mro_classes.push(start_class_id);
    mro_classes.extend_from_slice(start_class.mro());

    for &cls_id in &mro_classes {
        if let HeapData::Class(cls) = vm.heap.get(cls_id) {
            for (_, val) in cls.namespace() {
                if let Value::DefFunction(fid) = val {
                    if *fid == func_id {
                        vm.heap.inc_ref(cls_id);
                        return Some(Value::Ref(cls_id));
                    }
                } else if let Value::Ref(cid) = val {
                    match vm.heap.get(*cid) {
                        HeapData::Closure(closure) if closure.func_id == func_id => {
                            vm.heap.inc_ref(cls_id);
                            return Some(Value::Ref(cls_id));
                        }
                        HeapData::FunctionDefaults(fd) if fd.func_id == func_id => {
                            vm.heap.inc_ref(cls_id);
                            return Some(Value::Ref(cls_id));
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    None
}

fn validate_super_type(val: &Value, vm: &VM<'_>) -> RunResult<()> {
    match val {
        Value::Builtin(Builtins::Type(_)) => Ok(()),
        Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_) | HeapData::HostClassType(_)) => Ok(()),
        _ => {
            let got = val.py_type(vm).cpython_arg_name(vm.heap, vm.interns);
            Err(ExcType::type_error(format!(
                "super() argument 1 must be a type, not {got}"
            )))
        }
    }
}

fn type_or_class_name<'i>(val: &Value, vm: &'i VM<'_>) -> Cow<'i, str> {
    match val {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Class(_) => class_name(*id, vm.heap, vm.interns),
            HeapData::HostClassType(ty) => Cow::Borrowed(ty.name(vm.interns)),
            _ => val.py_type_name(vm),
        },
        Value::Builtin(Builtins::Type(t)) => t.name(vm.heap, vm.interns),
        _ => val.py_type_name(vm),
    }
}

fn validate_super_obj(type_arg: &Value, obj_arg: &Value, vm: &mut VM<'_>) -> RunResult<()> {
    if matches!(obj_arg, Value::None) {
        return Ok(());
    }
    if isinstance_check(obj_arg, type_arg, vm)? {
        return Ok(());
    }
    let is_type_or_class = matches!(obj_arg, Value::Builtin(Builtins::Type(_)))
        || matches!(obj_arg, Value::Ref(id) if matches!(vm.heap.get(*id), HeapData::Class(_) | HeapData::HostClassType(_)));
    if is_type_or_class {
        if issubclass_check(obj_arg, type_arg, vm)? {
            return Ok(());
        }
        let obj_name = type_or_class_name(obj_arg, vm);
        let type_name = type_or_class_name(type_arg, vm);
        return Err(ExcType::type_error(format!(
            "super(type, obj): obj (type {obj_name}) is not an instance or subtype of type ({type_name})."
        )));
    }
    let obj_name = obj_arg.py_type_name(vm);
    let type_name = type_or_class_name(type_arg, vm);
    Err(ExcType::type_error(format!(
        "super(type, obj): obj (instance of {obj_name}) is not an instance or subtype of type ({type_name})."
    )))
}

fn super_mro_lookup(type_arg: &Value, obj_arg: &Value, attr_str: &str, vm: &mut VM<'_>) -> Option<Value> {
    let self_class_id = match obj_arg {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Instance(inst) => inst.class(),
            HeapData::Class(_) => *id,
            _ => return None,
        },
        _ => return None,
    };

    let HeapData::Class(self_class) = vm.heap.get(self_class_id) else {
        return None;
    };

    let mut full_mro = Vec::with_capacity(1 + self_class.mro().len());
    full_mro.push(self_class_id);
    full_mro.extend_from_slice(self_class.mro());

    let target_type_id = match type_arg {
        Value::Ref(id) => *id,
        _ => return None,
    };

    let type_idx = full_mro.iter().position(|&id| id == target_type_id)?;

    for &ancestor_id in &full_mro[type_idx + 1..] {
        if let HeapData::Class(ancestor_cls) = vm.heap.get(ancestor_id)
            && let Some(member) = ancestor_cls.namespace().get_by_str(attr_str, vm.heap, vm.interns)
        {
            let member_val = member.clone_with_heap(vm.heap);
            if is_method_value(&member_val, vm) {
                let bound = BoundMethod {
                    instance: obj_arg.clone_with_heap(vm.heap),
                    func: member_val,
                };
                return Some(Value::Ref(vm.heap.allocate(HeapData::BoundMethod(bound))));
            }
            return Some(member_val);
        }
    }

    None
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, Super> {
    fn py_type(&self, _vm: &VM<'h>) -> Type {
        Type::Super
    }

    fn py_len(&self, _vm: &VM<'h>) -> Option<usize> {
        None
    }

    fn py_eq_impl(&self, _other: &Value, _vm: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, _vm: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self.id())))
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _heap_ids: &mut LazyHeapSet) -> RunResult<()> {
        let s = self.get(vm.heap);
        let type_name = type_or_class_name(&s.type_arg, vm);
        match &s.obj_arg {
            Some(obj) => {
                let obj_type_name = match obj {
                    Value::Ref(id) => match vm.heap.get(*id) {
                        HeapData::Class(_) => class_name(*id, vm.heap, vm.interns),
                        HeapData::HostClassType(ty) => Cow::Borrowed(ty.name(vm.interns)),
                        _ => obj.py_type_name(vm),
                    },
                    Value::Builtin(Builtins::Type(t)) => t.name(vm.heap, vm.interns),
                    _ => obj.py_type_name(vm),
                };
                write!(f, "<super: <class '{type_name}'>, <{obj_type_name} object>>")?;
            }
            None => {
                write!(f, "<super: <class '{type_name}'>, NULL>")?;
            }
        }
        Ok(())
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        let attr_str = attr.as_str(vm.interns);
        let s = self.get(vm.heap);

        if attr_str == "__thisclass__" {
            let val = s.type_arg.clone_with_heap(vm.heap);
            return Ok(Some(CallResult::Value(val)));
        }
        if attr_str == "__self__" {
            let val = match &s.obj_arg {
                Some(obj) => obj.clone_with_heap(vm.heap),
                None => Value::None,
            };
            return Ok(Some(CallResult::Value(val)));
        }
        if attr_str == "__self_class__" {
            let val = match &s.obj_arg {
                Some(obj) => match obj {
                    Value::Ref(id) => match vm.heap.get(*id) {
                        HeapData::Instance(inst) => {
                            let cid = inst.class();
                            vm.heap.inc_ref(cid);
                            Value::Ref(cid)
                        }
                        HeapData::Class(_) => {
                            vm.heap.inc_ref(*id);
                            Value::Ref(*id)
                        }
                        _ => Value::Builtin(Builtins::Type(obj.py_type(vm))),
                    },
                    _ => Value::Builtin(Builtins::Type(obj.py_type(vm))),
                },
                None => Value::None,
            };
            return Ok(Some(CallResult::Value(val)));
        }

        let Some(obj) = s.obj_arg.as_ref().map(|o| o.clone_with_heap(vm.heap)) else {
            return Err(ExcType::attribute_error("super", attr_str));
        };
        defer_drop!(obj, vm);

        let type_arg = self.get(vm.heap).type_arg.clone_with_heap(vm.heap);
        defer_drop!(type_arg, vm);

        if let Some(val) = super_mro_lookup(type_arg, obj, attr_str, vm) {
            return Ok(Some(CallResult::Value(val)));
        }

        Err(ExcType::attribute_error("super", attr_str))
    }

    fn py_call_attr(&mut self, vm: &mut VM<'h>, attr: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        let attr_str = attr.as_str(vm.interns);
        let s = self.get(vm.heap);
        let Some(obj) = s.obj_arg.as_ref().map(|o| o.clone_with_heap(vm.heap)) else {
            args.drop_with(vm.heap);
            return Err(ExcType::attribute_error("super", attr_str));
        };
        defer_drop!(obj, vm);

        let type_arg = self.get(vm.heap).type_arg.clone_with_heap(vm.heap);
        defer_drop!(type_arg, vm);

        if let Some(member) = super_mro_lookup(type_arg, obj, attr_str, vm) {
            defer_drop!(member, vm);
            return vm.call_function(member, args);
        }

        if attr_str == "__init__" {
            let pos = args.into_pos_only("object.__init__", vm.heap)?;
            if !pos.as_slice().is_empty() {
                pos.drop_with(vm.heap);
                return Err(ExcType::type_error(
                    "object.__init__() takes exactly one argument (the instance to initialize)",
                ));
            }
            return Ok(CallResult::Value(Value::None));
        }

        if attr_str == "__init_subclass__" {
            let (pos, kwargs) = args.into_parts();
            let pos_len = pos.as_slice().len();
            pos.drop_with(vm.heap);
            let has_kwargs = !kwargs.is_empty();
            kwargs.drop_with(vm.heap);
            let target_name = type_or_class_name(obj, vm);
            if pos_len > 0 {
                return Err(ExcType::type_error(format!(
                    "{target_name}.__init_subclass__() takes no arguments ({pos_len} given)"
                )));
            }
            if has_kwargs {
                return Err(ExcType::type_error(format!(
                    "{target_name}.__init_subclass__() takes no keyword arguments"
                )));
            }
            return Ok(CallResult::Value(Value::None));
        }

        args.drop_with(vm.heap);
        Err(ExcType::attribute_error("super", attr_str))
    }
}

impl HeapItem for Super {
    fn py_dec_ref_ids(&mut self, stack: &mut Vec<HeapId>) {
        self.type_arg.py_dec_ref_ids(stack);
        if let Some(obj) = &mut self.obj_arg {
            obj.py_dec_ref_ids(stack);
        }
    }
}
