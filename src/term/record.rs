//! Native records of OTP 29: the definition that the `Recs` chunk of a
//! module gives, and the record term that refers to it (`erl_record.h:41-71`).

use super::{Atom, AtomTable, Term, make_hash2};
use std::{cmp::Ordering, sync::Arc};

/// The position of the first key in the definition tuple, after the field
/// order, the hash, the module, the name, and `is_exported`.
const DEF_TUPLE_KEYS: usize = 5;

/// The definition of a native record, as `parse_record_chunk_data` makes it
/// (`beam_file.c:1077-1300`). Each record term of the definition shares it.
#[derive(Debug)]
pub(crate) struct RecordDef {
    /// The module that defines the record.
    pub(crate) module: Atom,
    /// The name of the record.
    pub(crate) name: Atom,
    /// Whether the module exports the record.
    pub(crate) is_exported: bool,
    /// The field names in flatmap key order (atom index order,
    /// `beam_file.c:1263-1265`).
    pub(crate) keys: Box<[Atom]>,
    /// The sorted index of each field in definition order
    /// (`beam_file.c:1288-1292`).
    pub(crate) order: Box<[usize]>,
    /// The default value of each sorted field, or `None` for a field with
    /// no default (`make_catch(0)`, `beam_file.c:1201-1202`).
    pub(crate) defaults: Box<[Option<Term>]>,
    /// The `phash2` of `{Module, Name, IsExported, Key1, ..., KeyN}` with
    /// the keys in definition order (`beam_file.c:1238-1256`).
    pub(crate) hash: u32,
    /// The ERTS definition tuple `{FieldOrder, Hash, Module, Name,
    /// IsExported, Key...}` (`erl_record.h:43-61`). `phash/2`, equality, and
    /// the flat size read it, as ERTS reads the definition as a tuple.
    pub(crate) tuple: Term,
    /// Whether only record terms hold the definition: `records:create/4`
    /// made it, and no module code has it. The live-heap count counts such
    /// a definition, as ERTS builds it on the process heap
    /// (`erl_record.c:645-656`).
    pub(crate) transient: bool,
}

impl RecordDef {
    /// Return the definition of record `name` of `module` with `fields` in
    /// definition order, as `parse_record_chunk_data` builds it
    /// (`beam_file.c:1223-1299`).
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let def = RecordDef::new(module, name, false, vec![(x, Some(Term::Integer(0))), (y, None)], &atoms);
    /// assert_eq!(def.keys.len(), 2);
    /// ```
    #[expect(
        clippy::fn_params_excessive_bools,
        reason = "`is_exported` is the Erlang boolean of a record definition (`beam_file.c:1223-1299`), stored and compared as data"
    )]
    pub(crate) fn new(
        module: Atom,
        name: Atom,
        is_exported: bool,
        fields: Vec<(Atom, Option<Term>)>,
        atoms: &AtomTable,
    ) -> Self {
        let exported = Term::from(is_exported);
        let mut hashed = vec![
            Term::Atom(module.clone()),
            Term::Atom(name.clone()),
            exported.clone(),
        ];
        hashed.extend(fields.iter().map(|(key, _)| Term::Atom(key.clone())));
        let hash = make_hash2(atoms, &Term::tuple(hashed)).0;
        let mut sorted: Vec<(usize, Atom, Option<Term>)> = fields
            .into_iter()
            .enumerate()
            .map(|(position, (key, value))| (position, key, value))
            .collect();
        sorted.sort_by_key(|(_, key, _)| key.index());
        let mut order = vec![0; sorted.len()];
        for (index, (position, _, _)) in sorted.iter().enumerate() {
            order[*position] = index;
        }
        let (keys, defaults): (Vec<Atom>, Vec<Option<Term>>) = sorted
            .into_iter()
            .map(|(_, key, value)| (key, value))
            .unzip();
        let order_tuple = Term::tuple(
            order
                .iter()
                .map(|&index| Term::Integer(index as i64))
                .collect(),
        );
        let mut tuple = vec![
            order_tuple,
            Term::Integer(i64::from(hash)),
            Term::Atom(module.clone()),
            Term::Atom(name.clone()),
            exported,
        ];
        tuple.extend(keys.iter().cloned().map(Term::Atom));
        RecordDef {
            module,
            name,
            is_exported,
            keys: keys.into(),
            order: order.into(),
            defaults: defaults.into(),
            hash,
            tuple: Term::tuple(tuple),
            transient: false,
        }
    }

    /// Return the field names as atom terms in the sorted key order: the
    /// `keys[]` of the ERTS definition tuple (`erl_record.h:58-60`).
    pub(crate) fn key_terms(&self) -> &[Term] {
        match &self.tuple {
            Term::Tuple(elements) => &elements[DEF_TUPLE_KEYS..],
            _ => unreachable!("a record definition is a tuple"),
        }
    }

    /// Return the sorted index of field `key`, or `None`.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// assert_eq!(def.index_of(&x), Some(0));
    /// ```
    pub(crate) fn index_of(&self, key: &Atom) -> Option<usize> {
        self.keys.iter().position(|field| field == key)
    }
}

/// The thing of a record term: the shared definition and the values of the
/// fields in the sorted key order (`ErtsRecordInstance`, `erl_record.h:63-71`).
#[derive(Debug)]
pub(super) struct RecordThing {
    def: Arc<RecordDef>,
    pub(super) values: Vec<Term>,
}

/// A native record term. Copies share the thing, as an ERTS copy is the
/// same boxed pointer. The thing is `None` only after `take_last` gave its
/// values to an iterative drop.
#[derive(Debug, Clone)]
pub struct Record(Option<Arc<RecordThing>>);

impl Record {
    /// Return a record of `def` with `values` in the sorted key order.
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// let record = Record::new(def, vec![Term::Integer(1), Term::Integer(2)]);
    /// assert_eq!(record.values().len(), 2);
    /// ```
    pub(crate) fn new(def: Arc<RecordDef>, values: Vec<Term>) -> Self {
        Record(Some(Arc::new(RecordThing { def, values })))
    }

    fn thing(&self) -> &RecordThing {
        self.0.as_deref().expect("a live record term has its thing")
    }

    /// Return the definition of the record.
    pub(crate) fn def(&self) -> &Arc<RecordDef> {
        &self.thing().def
    }

    /// Return the values in the sorted key order.
    pub(crate) fn values(&self) -> &[Term] {
        &self.thing().values
    }

    /// Return the shared thing of the record.
    pub(super) fn arc(&self) -> &Arc<RecordThing> {
        self.0.as_ref().expect("a live record term has its thing")
    }

    /// Return whether two records hold one thing.
    pub(super) fn same_thing(&self, other: &Record) -> bool {
        match (&self.0, &other.0) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Take the values, when this is the last holder of the thing, and
    /// leave the record empty, so a drop does not recurse.
    pub(super) fn take_last(&mut self) -> Option<Vec<Term>> {
        self.0
            .take()
            .and_then(Arc::into_inner)
            .map(|thing| thing.values)
    }

    /// Return the order of the definitions of two records that do not share
    /// one definition, as `erts_cmp_compound` compares them
    /// (`utils.c:1906-1939`): module, name, `is_exported`, field count, then
    /// the keys in definition order.
    pub(super) fn compare_defs(&self, other: &Record, atoms: &AtomTable) -> Ordering {
        let (left, right) = (self.def(), other.def());
        if Arc::ptr_eq(left, right) {
            return Ordering::Equal;
        }
        let name = |atom: &Atom| atoms.name_of(atom).to_owned();
        name(&left.module)
            .cmp(&name(&right.module))
            .then_with(|| name(&left.name).cmp(&name(&right.name)))
            .then(left.is_exported.cmp(&right.is_exported))
            .then(left.keys.len().cmp(&right.keys.len()))
            .then_with(|| {
                left.order
                    .iter()
                    .zip(right.order.iter())
                    .map(|(&l, &r)| (&left.keys[l], &right.keys[r]))
                    .find(|(l, r)| l != r)
                    .map_or(Ordering::Equal, |(l, r)| name(l).cmp(&name(r)))
            })
    }

    /// Return the pairs of values of two records in definition order.
    pub(super) fn value_pairs<'a>(
        &'a self,
        other: &'a Record,
    ) -> impl DoubleEndedIterator<Item = (&'a Term, &'a Term)> {
        let (left, right) = (self.thing(), other.thing());
        left.def
            .order
            .iter()
            .zip(right.def.order.iter())
            .map(|(&l, &r)| (&left.values[l], &right.values[r]))
    }
}

#[cfg(test)]
#[path = "../../tests/term/record.rs"]
mod tests;
