use crate::foundation::error::{WavyteError, WavyteResult};
use crate::scene::model::CompositionDef;
use crate::schema::validate::validate_composition;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// v0.3 composition boundary object.
///
/// This is the JSON-facing, human-edited representation of a scene graph. It is validated and
/// normalized into a compact runtime IR when constructing a [`crate::session::render_session::RenderSession`].
#[derive(Debug, Clone)]
pub struct Composition {
    def: CompositionDef,
}

impl Composition {
    /// Parse a v0.3 composition from a JSON reader.
    pub fn from_reader<R: std::io::Read>(r: R) -> WavyteResult<Self> {
        let def: CompositionDef = serde_json::from_reader(r)
            .map_err(|e| WavyteError::validation(format!("parse v0.3 composition JSON: {e}")))?;
        Ok(Self { def })
    }

    /// Parse a v0.3 composition from a JSON file on disk.
    pub fn from_path(path: impl AsRef<Path>) -> WavyteResult<Self> {
        let path = path.as_ref();
        let f = File::open(path).map_err(|e| {
            WavyteError::validation(format!("open composition JSON '{}': {e}", path.display()))
        })?;
        let r = BufReader::new(f);
        Self::from_reader(r)
    }

    /// Validate the composition against the v0.3 schema.
    pub fn validate(&self) -> WavyteResult<()> {
        validate_composition(&self.def)
            .map_err(|e| WavyteError::validation(format!("v0.3 schema validation failed: {e}")))
    }

    /// Return the declared duration (in frames).
    pub fn duration_frames(&self) -> u64 {
        self.def.duration
    }

    pub(crate) fn from_def(def: CompositionDef) -> Self {
        Self { def }
    }

    pub(crate) fn def(&self) -> &CompositionDef {
        &self.def
    }
}
