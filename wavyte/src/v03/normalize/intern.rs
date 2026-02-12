use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct InternId(pub(crate) u32);

#[derive(Debug, Default)]
pub(crate) struct StringInterner {
    ids_by_str: HashMap<String, InternId>,
    strs_by_id: Vec<String>,
}

impl StringInterner {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn intern(&mut self, s: &str) -> InternId {
        if let Some(&id) = self.ids_by_str.get(s) {
            return id;
        }
        let id = InternId(u32::try_from(self.strs_by_id.len()).unwrap());
        self.strs_by_id.push(s.to_owned());
        self.ids_by_str.insert(s.to_owned(), id);
        id
    }

    pub(crate) fn get(&self, id: InternId) -> &str {
        &self.strs_by_id[id.0 as usize]
    }
}
