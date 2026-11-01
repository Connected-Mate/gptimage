//! `hoi4.image_audit`: which sprites and textures the mod uses, for which family, and whether the art fits.
//!
//! The index, the trace and the reuse findings come from the shared HOI4 engine; the canvas and card-corner
//! checks come from the family profiles. Read only: it opens the mod and the game and writes nothing.

use imagegen_core::audit::{self, AuditInput, DEFAULT_ALLOWLIST, DEFAULT_LIMIT};
use imagegen_core::error::{Error as CoreError, RetryClass};
use serde_json::Value;

use crate::api::AuditParams;

use super::{EResult, Engine, EngineError};

impl Engine {
    /// The allowlist file in force: the named one (which must exist) or the project default when present.
    pub(crate) fn audit_allowlist(&self, named: Option<&str>) -> EResult<Option<std::path::PathBuf>> {
        match named {
            Some(rel) => {
                let path = self.fs.resolve(rel)?;
                if !path.is_file() {
                    return Err(EngineError::new("not_found", format!("allowlist {rel} does not exist in the project"), RetryClass::Terminal));
                }
                Ok(Some(path))
            }
            None => {
                let path = self.fs.resolve(DEFAULT_ALLOWLIST)?;
                Ok(path.is_file().then_some(path))
            }
        }
    }

    /// Audit the sprites and textures the mod uses.
    pub fn audit(&self, p: AuditParams) -> EResult<Value> {
        let allowlist = self.audit_allowlist(p.allowlist.as_deref())?;
        let game = self.game_dir();
        let limit = p.limit.map(|l| l as usize).unwrap_or(DEFAULT_LIMIT);
        if limit == 0 {
            return Err(EngineError::invalid("limit must be at least 1"));
        }
        let families = p.families.unwrap_or_default();
        let report = audit::run_audit(&AuditInput {
            project_root: self.fs.root(),
            game_dir: game.as_deref(),
            allowlist: allowlist.as_deref(),
            families: &families,
            limit,
            profiles: &self.profiles,
        })
        .map_err(|e| match e {
            CoreError::Invalid(m) => EngineError::invalid(m),
            other => EngineError::from(other),
        })?;
        serde_json::to_value(report).map_err(|e| EngineError::new("internal", e.to_string(), RetryClass::Terminal))
    }
}
