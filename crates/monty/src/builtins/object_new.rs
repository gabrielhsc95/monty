//! Implementation of `object.__new__`, the instance allocator.

use monty_types::ExcType;

use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcTypeExt, RunResult},
    heap::{DropWithContext, HeapData},
    intern::StaticStrings,
    types::{Dict, Instance, allocate_tuple},
    value::Value,
};

/// `object.__new__(cls, *args, **kwargs)` — allocates a new empty instance of `cls`.
pub fn builtin_object_new(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (mut pos, kwargs) = args.into_parts();
    kwargs.drop_with(vm.heap);
    let Some(cls_val) = pos.next() else {
        return Err(ExcType::type_error("object.__new__(): not enough arguments"));
    };
    defer_drop!(cls_val, vm);
    let extra_args: Vec<Value> = pos.collect();

    let Value::Ref(class_id) = cls_val else {
        extra_args.drop_with(vm.heap);
        let type_name = cls_val.py_type_name(vm);
        return Err(ExcType::type_error(format!(
            "object.__new__(X): X is not a type object ({type_name})"
        )));
    };

    let HeapData::Class(cls) = vm.heap.get(*class_id) else {
        extra_args.drop_with(vm.heap);
        let type_name = cls_val.py_type_name(vm);
        return Err(ExcType::type_error(format!(
            "object.__new__(X): X is not a type object ({type_name})"
        )));
    };

    let base_exc = cls.base_exception();
    let attrs = if base_exc.is_some() {
        let args_tuple = allocate_tuple(extra_args.into(), vm.heap);
        Dict::from_pairs(
            vec![(
                Value::InternString(vm.interns.intern_static(StaticStrings::Args)),
                args_tuple,
            )],
            vm,
        )?
    } else {
        extra_args.drop_with(vm.heap);
        Dict::new()
    };

    vm.heap.inc_ref(*class_id);
    let inst = Instance::new(*class_id, attrs);
    let inst_id = vm.heap.allocate(HeapData::Instance(Box::new(inst)));
    Ok(Value::Ref(inst_id))
}
