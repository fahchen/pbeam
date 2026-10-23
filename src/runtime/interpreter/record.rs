//! The native record instructions of OTP 29 (`erl_record.c:214-568`).

use std::sync::Arc;

use crate::term::{Atom, Record, RecordDef, Term};

/// Return the reason `{Tag, {{Module, Name}, Field}}` of a field error of a
/// record of `def` (`erl_record.c:330-337`, 437-443, 502-508).
///
/// # Examples
///
/// ```rust,ignore
/// let reason = field_reason("badfield", &def, Term::Atom(zz));
/// ```
pub(super) fn field_reason(tag: &str, def: &RecordDef, field: Term) -> Term {
    let id = Term::tuple(vec![
        Term::Atom(def.module.clone()),
        Term::Atom(def.name.clone()),
    ]);
    Term::tuple(vec![Term::static_atom(tag), Term::tuple(vec![id, field])])
}

/// Return the reason `{badrecord, Value}`.
pub(super) fn badrecord(value: Term) -> Term {
    Term::tuple(vec![Term::static_atom("badrecord"), value])
}

/// Return a new record of `def`, or the reason of its error, as
/// `erl_create_local_native_record` does (`erl_record.c:254-342`). The walk
/// takes each sorted key in turn: the next update when its field is the key,
/// or else the default. A field with no default gives the first `novalue`
/// error, and an update field that the walk does not take gives `badfield`,
/// which wins over `novalue` (`erl_record.c:319-325`). `updates` is in the
/// sorted key order.
///
/// # Examples
///
/// ```rust,ignore
/// let record = create(&def, vec![(&x, Term::Integer(1))])?;
/// ```
pub(super) fn create(def: &Arc<RecordDef>, updates: Vec<(&Atom, Term)>) -> Result<Term, Term> {
    let mut updates = updates.into_iter().peekable();
    let mut novalue = None;
    let mut values = Vec::with_capacity(def.keys.len());
    for (key, default) in def.keys.iter().zip(&def.defaults) {
        match updates.next_if(|(field, _)| *field == key) {
            Some((_, value)) => values.push(value),
            None => values.push(default.clone().unwrap_or_else(|| {
                novalue.get_or_insert(key);
                Term::Nil
            })),
        }
    }
    match (updates.next(), novalue) {
        (Some((field, _)), _) => Err(field_reason("badfield", def, Term::Atom(field.clone()))),
        (None, Some(key)) => Err(field_reason("novalue", def, Term::Atom(key.clone()))),
        (None, None) => Ok(Term::Record(Record::new(Arc::clone(def), values))),
    }
}

/// Return a copy of `record` with the updated fields, or the reason of its
/// error, as `erl_update_native_record` does (`erl_record.c:381-450`).
/// `updates` is in the sorted key order.
///
/// # Examples
///
/// ```rust,ignore
/// let record = update(&record, vec![(&x, Term::Integer(1))])?;
/// ```
pub(super) fn update(record: &Record, updates: Vec<(&Atom, Term)>) -> Result<Term, Term> {
    let def = record.def();
    let mut updates = updates.into_iter().peekable();
    let values = def
        .keys
        .iter()
        .zip(record.values())
        .map(
            |(key, old)| match updates.next_if(|(field, _)| *field == key) {
                Some((_, value)) => value,
                None => old.clone(),
            },
        )
        .collect();
    match updates.next() {
        Some((field, _)) => Err(field_reason("badfield", def, Term::Atom(field.clone()))),
        None => Ok(Term::Record(Record::new(Arc::clone(def), values))),
    }
}

/// Return the value of `field` of `value`, or the reason of its error, as
/// `erl_get_local_record_field` and `erl_get_record_field` do
/// (`erl_record.c:470-568`). `name` is the record name, or `'_'` for any
/// record. With `module`, the record must also be of that module and
/// exported.
///
/// # Examples
///
/// ```rust,ignore
/// let value = get_field(&record_term, None, &r, &x)?;
/// ```
pub(super) fn get_field(
    value: &Term,
    module: Option<&Atom>,
    name: &Atom,
    field: &Atom,
) -> Result<Term, Term> {
    let Term::Record(record) = value else {
        return Err(badrecord(value.clone()));
    };
    let def = record.def();
    let wildcard = module.is_none() && name.is_static("_");
    let accessible = match module {
        Some(module) => def.module == *module && def.name == *name && def.is_exported,
        None => wildcard || def.name == *name,
    };
    if !accessible {
        return Err(badrecord(value.clone()));
    }
    def.index_of(field)
        .map(|index| record.values()[index].clone())
        .ok_or_else(|| field_reason("badfield", def, Term::Atom(field.clone())))
}
