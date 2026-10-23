//! Bodies of the `records` BIFs of OTP 29 (`erl_record.c:570-1092`).

use std::sync::Arc;

use super::BifResult::Value;
use super::*;
use crate::runtime::interpreter::record::{badrecord, field_reason};
use crate::term::{AtomTable, Record, RecordDef, flatmap_order};

/// Return an `error` exception with `reason`.
fn raise(reason: Term) -> BifResult {
    BifResult::Raise {
        class: ExceptionClass::Error,
        reason,
    }
}

/// Return the record of `value`, or raise `badarg`, as `records_get_module_1`,
/// `records_get_name_1`, and `records_get_2` do (`erl_record.c:946-948`,
/// 967-969, 978-980).
fn record_or_badarg(value: &Term) -> Result<&Record, BifResult> {
    value.record().ok_or_else(BifResult::badarg)
}

/// Run `records:get_module/1` and `records:get_name/1`: return the module or
/// the name of a record (`erl_record.c:964-984`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(identity(&record, true), BifResult::Value(_)));
/// ```
pub(super) fn identity(value: &Term, module: bool) -> BifResult {
    match record_or_badarg(value) {
        Ok(record) => {
            let def = record.def();
            Value(Term::Atom(
                if module { &def.module } else { &def.name }.clone(),
            ))
        }
        Err(error) => error,
    }
}

/// Run `records:get/2`: return the value of field `key`, or raise `badarg`
/// when the term is not a record, the key is not an atom, or the record has
/// no such field (`erl_record.c:935-962`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(get(&x, &record), BifResult::Value(_)));
/// ```
pub(super) fn get(key: &Term, value: &Term) -> BifResult {
    let record = match record_or_badarg(value) {
        Ok(record) => record,
        Err(error) => return error,
    };
    let Term::Atom(key) = key else {
        return BifResult::badarg();
    };
    record
        .def()
        .index_of(key)
        .map_or_else(BifResult::badarg, |index| {
            Value(record.values()[index].clone())
        })
}

/// Run `records:get_field_names/1`: return the field names in definition
/// order, or raise `{badrecord, Term}` (`erl_record.c:986-1014`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(get_field_names(&record), BifResult::Value(_)));
/// ```
pub(super) fn get_field_names(value: &Term) -> BifResult {
    let Some(record) = value.record() else {
        return raise(badrecord(value.clone()));
    };
    let def = record.def();
    Value(Term::list_from(
        def.order
            .iter()
            .map(|&index| Term::Atom(def.keys[index].clone())),
    ))
}

/// Run `records:is_exported/1`: return whether the module exports the
/// record, or raise `{badrecord, Term}` (`erl_record.c:1016-1028`).
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(is_exported(&record), BifResult::Value(_)));
/// ```
pub(super) fn is_exported(value: &Term) -> BifResult {
    match value.record() {
        Some(record) => Value(Term::static_atom(if record.def().is_exported {
            "true"
        } else {
            "false"
        })),
        None => raise(badrecord(value.clone())),
    }
}

/// Run `records:get_definition/2`: return `{#{is_exported => B}, Fields}`
/// for record `name` of the current code of `module`, with each field in
/// definition order as `{Field, Default}`, or as `Field` when it has no
/// default. Raise `badarg` when an argument is not an atom or the current
/// code defines no such record (`erl_record.c:1030-1092`). The definition
/// need not be exported, and no module loads.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(get_definition(cx, &module, &name), BifResult::Value(_)));
/// ```
pub(super) fn get_definition(cx: &mut BifContext<'_>, module: &Term, name: &Term) -> BifResult {
    let (Term::Atom(module), Term::Atom(name)) = (module, name) else {
        return BifResult::badarg();
    };
    let scheduler = &cx.scheduler;
    let Some(def) = scheduler
        .code
        .get(scheduler.atoms.name_of(module))
        .and_then(|code| code.records.iter().find(|def| def.name == *name))
    else {
        return BifResult::badarg();
    };
    let fields = def.order.iter().map(|&index| {
        let key = Term::Atom(def.keys[index].clone());
        match &def.defaults[index] {
            Some(default) => Term::tuple(vec![key, default.clone()]),
            None => key,
        }
    });
    let options = exported_map(def.is_exported, &scheduler.atoms);
    Value(Term::tuple(vec![options, Term::list_from(fields)]))
}

/// Return the map `#{is_exported => Exported}`.
fn exported_map(exported: bool, atoms: &AtomTable) -> Term {
    let value = Term::static_atom(if exported { "true" } else { "false" });
    Map::from_pairs(vec![(Term::static_atom("is_exported"), value)], atoms)
        .map(Term::Map)
        .expect("an atom key fits a map")
}

/// Run `records:create/4`: return a new record of `module` and `name` with
/// the fields of the list in definition order, as `records_create_4` does
/// (`erl_record.c:592-773`). The checks run in the ERTS order: a module or
/// name that is not an atom raises `{badrecord, Name}`; options that are not
/// a map raise `{badmap, Options}`; options other than
/// `#{is_exported => true | false}` and an improper list raise `badarg`;
/// then each element in list order raises `badarg` when it is not a pair or
/// `{badfield, {{Module, Name}, Key}}` when its key is not an atom; then a
/// duplicate key raises `badarg`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(create(&atoms, &m, &n, &fields, &opts), BifResult::Value(_)));
/// ```
///
/// # Errors
///
/// Return a runtime error when the sort of the keys fails.
pub(super) fn create(
    atoms: &AtomTable,
    module: &Term,
    name: &Term,
    fields: &Term,
    options: &Term,
) -> BifResult {
    let (Term::Atom(module), Term::Atom(name)) = (module, name) else {
        return raise(badrecord(name.clone()));
    };
    let Term::Map(options_map) = options else {
        return raise(Term::tuple(vec![
            Term::static_atom("badmap"),
            options.clone(),
        ]));
    };
    let exported = match options_map.get(&Term::static_atom("is_exported")) {
        Some(Term::Atom(value)) if options_map.len() == 1 && value.is_static("true") => true,
        Some(Term::Atom(value)) if options_map.len() == 1 && value.is_static("false") => false,
        _ => return BifResult::badarg(),
    };
    let Some(elements) = fields.proper_list() else {
        return BifResult::badarg();
    };
    let mut pairs = Vec::new();
    for element in elements {
        let Term::Tuple(pair) = element else {
            return BifResult::badarg();
        };
        let [key, value] = &pair[..] else {
            return BifResult::badarg();
        };
        let Term::Atom(key) = key else {
            let id = Term::tuple(vec![Term::Atom(module.clone()), Term::Atom(name.clone())]);
            return raise(Term::tuple(vec![
                Term::static_atom("badfield"),
                Term::tuple(vec![id, key.clone()]),
            ]));
        };
        pairs.push((key.clone(), value.clone()));
    }
    let mut sorted: Vec<&Atom> = pairs.iter().map(|(key, _)| key).collect();
    sorted.sort_by_key(|key| key.index());
    if sorted.windows(2).any(|keys| keys[0] == keys[1]) {
        return BifResult::badarg();
    }
    let (keys, values): (Vec<Atom>, Vec<Term>) = pairs.into_iter().unzip();
    let def = RecordDef::new(
        module.clone(),
        name.clone(),
        exported,
        keys.into_iter().map(|key| (key, None)).collect(),
        atoms,
    );
    let mut sorted_values = vec![Term::Nil; values.len()];
    for (position, value) in values.into_iter().enumerate() {
        sorted_values[def.order[position]] = value;
    }
    Value(Term::Record(Record::new(Arc::new(def), sorted_values)))
}

/// Run `records:update/4`: return a copy of the record with the fields of
/// the map, as `records_update_4` does (`erl_record.c:775-933`). A term
/// that is not a record raises `{badrecord, Term}`. Unless `name` is `'_'`,
/// a module or name that is not an atom or that differs from the record
/// raises `{badrecord, Name}`. A term that is not a map raises
/// `{badmap, Map}`; an empty map gives the record. The first key in the
/// flatmap key order that is not a field raises
/// `{badfield, {{Module, Name}, Key}}`.
///
/// # Examples
///
/// ```rust,ignore
/// assert!(matches!(update(&atoms, &record, &m, &n, &map), BifResult::Value(_)));
/// ```
///
pub(super) fn update(
    atoms: &AtomTable,
    value: &Term,
    module: &Term,
    name: &Term,
    map: &Term,
) -> BifResult {
    let Some(record) = value.record() else {
        return raise(badrecord(value.clone()));
    };
    let def = record.def();
    if !matches!(name, Term::Atom(atom) if atom.is_static("_")) {
        let matches = matches!(
            (module, name),
            (Term::Atom(module), Term::Atom(name)) if def.module == *module && def.name == *name
        );
        if !matches {
            return raise(badrecord(name.clone()));
        }
    }
    let Term::Map(entries) = map else {
        return raise(Term::tuple(vec![Term::static_atom("badmap"), map.clone()]));
    };
    if entries.len() == 0 {
        return Value(value.clone());
    }
    let mut pairs: Vec<(Term, Term)> = entries
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    pairs.sort_by(|(left, _), (right, _)| {
        flatmap_order(left, right, atoms).unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut values = record.values().to_vec();
    for (key, new) in pairs {
        let index = match &key {
            Term::Atom(field) => def.index_of(field),
            _ => None,
        };
        match index {
            Some(index) => values[index] = new,
            None => return raise(field_reason("badfield", def, key)),
        }
    }
    Value(Term::Record(Record::new(Arc::clone(def), values)))
}
