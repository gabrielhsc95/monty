//! Implementation of `type.mro`, which returns a type's method resolution order as a list.

use monty_types::ExcType;

use crate::{
    args::ArgValues,
    builtins::{Builtins, BuiltinsFunctions},
    bytecode::VM,
    defer_drop,
    exception_private::{ExcTypeExt, RunResult},
    heap::{DropWithContext, HeapData},
    types::{List, Type, class::class_mro_values},
    value::Value,
};

/// `type.mro(cls, /)` — returns method resolution order as a list.
pub fn builtin_type_mro(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let (mut pos, kwargs) = args.into_parts();
    if !kwargs.is_empty() {
        pos.drop_with(vm.heap);
        kwargs.drop_with(vm.heap);
        return Err(ExcType::type_error("type.mro() takes no keyword arguments"));
    }
    let Some(cls_val) = pos.next() else {
        return Err(ExcType::type_error("unbound method type.mro() needs an argument"));
    };
    defer_drop!(cls_val, vm);
    let extra_pos: Vec<Value> = pos.collect();
    if !extra_pos.is_empty() {
        let count = extra_pos.len();
        extra_pos.drop_with(vm.heap);
        let type_name = match &cls_val {
            Value::Ref(id) if let HeapData::Class(cls) = vm.heap.get(*id) => cls.name().as_str(vm.interns).to_owned(),
            Value::Builtin(Builtins::ExcType(e)) => {
                let name: &'static str = (*e).into();
                name.rsplit_once('.').map_or(name, |(_, b)| b).to_owned()
            }
            Value::Builtin(Builtins::Type(t)) => t.dunder_name(vm.heap, vm.interns).into_owned(),
            Value::Builtin(Builtins::Function(BuiltinsFunctions::Type)) => "type".to_owned(),
            _ => cls_val.py_type_name(vm).into_owned(),
        };
        return Err(ExcType::type_error(format!(
            "{type_name}.mro() takes no arguments ({count} given)"
        )));
    }
    match cls_val {
        Value::Builtin(Builtins::Function(BuiltinsFunctions::Type)) => {
            let items = vec![
                Value::Builtin(Builtins::Function(BuiltinsFunctions::Type)),
                Value::Builtin(Builtins::Type(Type::Object)),
            ];
            let list_id = vm.heap.allocate(HeapData::List(List::new(items)));
            Ok(Value::Ref(list_id))
        }
        Value::Ref(id) if let HeapData::Class(cls) = vm.heap.get(*id) => {
            let class_id = *id;
            let mro = cls.mro().to_vec();
            let items = class_mro_values(class_id, &mro, vm.heap);
            let list_id = vm.heap.allocate(HeapData::List(List::new(items)));
            Ok(Value::Ref(list_id))
        }
        Value::Builtin(Builtins::Type(t)) => {
            let items = if *t == Type::Object {
                vec![Value::Builtin(Builtins::Type(Type::Object))]
            } else if *t == Type::Bool {
                vec![
                    Value::Builtin(Builtins::Type(Type::Bool)),
                    Value::Builtin(Builtins::Type(Type::Int)),
                    Value::Builtin(Builtins::Type(Type::Object)),
                ]
            } else if *t == Type::DateTime {
                vec![
                    Value::Builtin(Builtins::Type(Type::DateTime)),
                    Value::Builtin(Builtins::Type(Type::Date)),
                    Value::Builtin(Builtins::Type(Type::Object)),
                ]
            } else if matches!(*t, Type::DefaultDict | Type::Counter) {
                vec![
                    Value::Builtin(Builtins::Type(*t)),
                    Value::Builtin(Builtins::Type(Type::Dict)),
                    Value::Builtin(Builtins::Type(Type::Object)),
                ]
            } else {
                vec![
                    Value::Builtin(Builtins::Type(*t)),
                    Value::Builtin(Builtins::Type(Type::Object)),
                ]
            };
            let list_id = vm.heap.allocate(HeapData::List(List::new(items)));
            Ok(Value::Ref(list_id))
        }
        Value::Builtin(Builtins::ExcType(exc_type)) => {
            let mut items: Vec<Value> = exc_type
                .mro()
                .into_iter()
                .map(|b| Value::Builtin(Builtins::ExcType(b)))
                .collect();
            items.push(Value::Builtin(Builtins::Type(Type::Object)));
            let list_id = vm.heap.allocate(HeapData::List(List::new(items)));
            Ok(Value::Ref(list_id))
        }
        other => {
            let type_name = other.py_type_name(vm);
            Err(ExcType::type_error(format!(
                "descriptor 'mro' for 'type' objects doesn't apply to a '{type_name}' object"
            )))
        }
    }
}
