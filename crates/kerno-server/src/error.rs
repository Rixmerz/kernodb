//! One error type for both transports: a [`Kind`] (which fixes the HTTP status, the gRPC code and
//! the machine-readable `code`) and a message the client can read.

use kerno::Error as UnitError;

/// Why a call failed. Each kind is one row of the table in the README ("Errors").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The request is malformed or asks for something the database refuses (HTTP 400).
    InvalidRequest,
    /// The unit type's schema refuses the write (HTTP 422).
    SchemaViolation,
    /// The request carries no token, or one the server does not know (HTTP 401).
    Unauthenticated,
    /// No such unit, relation, schema or route (HTTP 404).
    NotFound,
    /// The route exists but not for this method (HTTP 405).
    MethodNotAllowed,
    /// `expected_revision` is not the unit's revision (HTTP 409).
    RevisionConflict,
    /// The unit's state forbids the change: it still has relations, or it is deleted (HTTP 409).
    Conflict,
    /// The payload or the request is over the limit (HTTP 413).
    PayloadTooLarge,
    /// The database cannot answer right now (HTTP 503).
    Unavailable,
    /// The server failed; the cause is logged, not sent (HTTP 500).
    Internal,
}

impl Kind {
    /// The `error.code` of the JSON body.
    pub fn code(self) -> &'static str {
        match self {
            Kind::InvalidRequest => "invalid_request",
            Kind::SchemaViolation => "schema_violation",
            Kind::Unauthenticated => "unauthenticated",
            Kind::NotFound => "not_found",
            Kind::MethodNotAllowed => "method_not_allowed",
            Kind::RevisionConflict => "revision_conflict",
            Kind::Conflict => "conflict",
            Kind::PayloadTooLarge => "payload_too_large",
            Kind::Unavailable => "unavailable",
            Kind::Internal => "internal",
        }
    }

    pub fn http_status(self) -> u16 {
        match self {
            Kind::InvalidRequest => 400,
            Kind::SchemaViolation => 422,
            Kind::Unauthenticated => 401,
            Kind::NotFound => 404,
            Kind::MethodNotAllowed => 405,
            Kind::RevisionConflict | Kind::Conflict => 409,
            Kind::PayloadTooLarge => 413,
            Kind::Unavailable => 503,
            Kind::Internal => 500,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct ApiError {
    pub kind: Kind,
    pub message: String,
    /// The `Allow` header of a 405.
    pub allow: Option<&'static str>,
    /// What an `Internal` error hides from the client: logged by [`ApiError::log`].
    detail: Option<String>,
}

impl ApiError {
    pub fn new(kind: Kind, message: impl Into<String>) -> Self {
        ApiError { kind, message: message.into(), allow: None, detail: None }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Kind::InvalidRequest, message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(Kind::NotFound, message)
    }

    pub fn method_not_allowed(allow: &'static str) -> Self {
        ApiError { allow: Some(allow), ..Self::new(Kind::MethodNotAllowed, format!("this route accepts {allow}")) }
    }

    /// A failure of the server. The client gets a fixed message: the cause may name files.
    pub fn internal(detail: impl Into<String>) -> Self {
        ApiError { detail: Some(detail.into()), ..Self::new(Kind::Internal, "internal error") }
    }

    /// The one answer to every request without a valid token: missing, malformed and unknown
    /// tokens are not told apart, so a caller learns nothing by probing.
    pub fn unauthenticated() -> Self {
        Self::new(Kind::Unauthenticated, "a valid API token is required: send `Authorization: Bearer <token>`")
    }

    /// Logs the hidden cause of an `Internal` error. Both transports call it once per failed call.
    pub fn log(&self) {
        if let Some(detail) = &self.detail {
            tracing::error!(detail = %detail, "internal error");
        }
    }

    /// The JSON body of every failed HTTP call.
    pub fn body(&self) -> Vec<u8> {
        serde_json::json!({ "error": { "code": self.kind.code(), "message": self.message } }).to_string().into_bytes()
    }
}

impl From<UnitError> for ApiError {
    fn from(e: UnitError) -> Self {
        let kind = match &e {
            UnitError::NotFound { .. } | UnitError::RelationNotFound { .. } => Kind::NotFound,
            UnitError::InvalidArgument(_)
            | UnitError::SchemaExists { .. }
            | UnitError::NotIndexed { .. }
            | UnitError::UnknownType { .. }
            | UnitError::NotUnique { .. }
            | UnitError::BadEndpoint { .. } => Kind::InvalidRequest,
            UnitError::SchemaViolation { .. } => Kind::SchemaViolation,
            UnitError::Conflict { .. } => Kind::RevisionConflict,
            UnitError::Deleted { .. } | UnitError::UnitHasRelations { .. } | UnitError::AlreadyExists { .. } => {
                Kind::Conflict
            }
            UnitError::PayloadTooLarge { .. } | UnitError::ChangeSetTooLarge { .. } => Kind::PayloadTooLarge,
            UnitError::IndexNotReady { .. } => Kind::Unavailable,
            // Storage errors, damage, and any variant added later: the server's problem.
            _ => return ApiError::internal(e.to_string()),
        };
        ApiError::new(kind, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use kerno::units::{HistoryPolicy, IndexState, RevNo, UnitId};
    use kerno::{Error, RelationId};

    use super::*;

    const UNIT: UnitId = UnitId([7; 16]);
    const RELATION: RelationId = RelationId([9; 16]);

    fn kind(e: Error) -> Kind {
        ApiError::from(e).kind
    }

    #[test]
    fn every_unit_error_maps_to_its_kind() {
        let io = || Error::Storage(kerno_core::Error::Io(std::io::Error::other("disk")));
        let cases = [
            (Error::NotFound { unit: UNIT }, Kind::NotFound),
            (Error::RelationNotFound { relation: RELATION }, Kind::NotFound),
            (Error::InvalidArgument("x".into()), Kind::InvalidRequest),
            (Error::SchemaExists { unit_type: "t".into() }, Kind::InvalidRequest),
            (Error::NotIndexed { detail: "x".into() }, Kind::InvalidRequest),
            (Error::UnknownType { unit_type: "t".into() }, Kind::InvalidRequest),
            (Error::NotUnique { detail: "x".into() }, Kind::InvalidRequest),
            (Error::BadEndpoint { detail: "x".into() }, Kind::InvalidRequest),
            (Error::SchemaViolation { detail: "x".into() }, Kind::SchemaViolation),
            (Error::Conflict { unit: UNIT, expected: RevNo(1), found: RevNo(2) }, Kind::RevisionConflict),
            (Error::Deleted { unit: UNIT }, Kind::Conflict),
            (Error::UnitHasRelations { unit: UNIT, relation: RELATION }, Kind::Conflict),
            (Error::AlreadyExists { unit: UNIT }, Kind::Conflict),
            (Error::PayloadTooLarge { len: 2, max: 1 }, Kind::PayloadTooLarge),
            (Error::ChangeSetTooLarge { ops: 2, bytes: 2 }, Kind::PayloadTooLarge),
            (Error::IndexNotReady { state: IndexState::Stale, watermark: None }, Kind::Unavailable),
            (io(), Kind::Internal),
            (Error::HistoryUnavailable { policy: HistoryPolicy::None }, Kind::Internal),
            (Error::Pruned { unit: UNIT, oldest: RevNo(3) }, Kind::Internal),
            (Error::ChangeSetClosed, Kind::Internal),
        ];
        for (error, expected) in cases {
            let shown = error.to_string();
            assert_eq!(kind(error), expected, "{shown}");
        }
    }

    #[test]
    fn a_kind_fixes_its_status_and_code() {
        let table = [
            (Kind::InvalidRequest, 400, "invalid_request"),
            (Kind::SchemaViolation, 422, "schema_violation"),
            (Kind::Unauthenticated, 401, "unauthenticated"),
            (Kind::NotFound, 404, "not_found"),
            (Kind::MethodNotAllowed, 405, "method_not_allowed"),
            (Kind::RevisionConflict, 409, "revision_conflict"),
            (Kind::Conflict, 409, "conflict"),
            (Kind::PayloadTooLarge, 413, "payload_too_large"),
            (Kind::Unavailable, 503, "unavailable"),
            (Kind::Internal, 500, "internal"),
        ];
        for (kind, status, code) in table {
            assert_eq!((kind.http_status(), kind.code()), (status, code));
        }
    }

    #[test]
    fn an_internal_error_never_sends_its_cause() {
        let e = ApiError::from(Error::Storage(kerno_core::Error::Io(std::io::Error::other("/secret/path"))));
        assert_eq!(e.kind, Kind::Internal);
        assert_eq!(e.message, "internal error");
        assert!(!String::from_utf8(e.body()).unwrap().contains("/secret/path"));
        // A client-caused error keeps its message.
        let e = ApiError::from(Error::InvalidArgument("limit 0 is outside 1..=10".into()));
        assert!(e.message.contains("limit 0 is outside 1..=10"));
    }

    #[test]
    fn the_body_has_one_shape() {
        let body: serde_json::Value = serde_json::from_slice(&ApiError::not_found("no unit").body()).unwrap();
        assert_eq!(body, serde_json::json!({ "error": { "code": "not_found", "message": "no unit" } }));
    }
}
