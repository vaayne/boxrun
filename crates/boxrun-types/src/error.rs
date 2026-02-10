use std::fmt;

/// Error codes matching the Python BoxRun error codes exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorCode {
    BoxNotFound,
    BoxNotRunning,
    BoxAlreadyRunning,
    ExecNotFound,
    ExecAlreadyFinished,
    ImagePullFailed,
    NameAlreadyExists,
    ResourceExceeded,
    Timeout,
    RuntimeError,
    Canceled,
}

impl ErrorCode {
    /// Return the string representation matching the Python error codes.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BoxNotFound => "BOX_NOT_FOUND",
            Self::BoxNotRunning => "BOX_NOT_RUNNING",
            Self::BoxAlreadyRunning => "BOX_ALREADY_RUNNING",
            Self::ExecNotFound => "EXEC_NOT_FOUND",
            Self::ExecAlreadyFinished => "EXEC_ALREADY_FINISHED",
            Self::ImagePullFailed => "IMAGE_PULL_FAILED",
            Self::NameAlreadyExists => "NAME_ALREADY_EXISTS",
            Self::ResourceExceeded => "RESOURCE_EXCEEDED",
            Self::Timeout => "TIMEOUT",
            Self::RuntimeError => "RUNTIME_ERROR",
            Self::Canceled => "CANCELED",
        }
    }

    /// Map error code to HTTP status code.
    pub fn http_status(&self) -> u16 {
        match self {
            Self::BoxNotFound | Self::ExecNotFound => 404,
            Self::ResourceExceeded | Self::NameAlreadyExists | Self::BoxAlreadyRunning => 409,
            _ => 400,
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The main error type for BoxRun, carrying an error code and message.
#[derive(Debug, Clone)]
pub struct BoxRunError {
    pub code: ErrorCode,
    pub message: String,
}

impl BoxRunError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn http_status(&self) -> u16 {
        self.code.http_status()
    }
}

impl fmt::Display for BoxRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BoxRunError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_strings() {
        assert_eq!(ErrorCode::BoxNotFound.as_str(), "BOX_NOT_FOUND");
        assert_eq!(ErrorCode::BoxNotRunning.as_str(), "BOX_NOT_RUNNING");
        assert_eq!(ErrorCode::BoxAlreadyRunning.as_str(), "BOX_ALREADY_RUNNING");
        assert_eq!(ErrorCode::ExecNotFound.as_str(), "EXEC_NOT_FOUND");
        assert_eq!(
            ErrorCode::ExecAlreadyFinished.as_str(),
            "EXEC_ALREADY_FINISHED"
        );
        assert_eq!(ErrorCode::ImagePullFailed.as_str(), "IMAGE_PULL_FAILED");
        assert_eq!(ErrorCode::NameAlreadyExists.as_str(), "NAME_ALREADY_EXISTS");
        assert_eq!(ErrorCode::ResourceExceeded.as_str(), "RESOURCE_EXCEEDED");
        assert_eq!(ErrorCode::Timeout.as_str(), "TIMEOUT");
        assert_eq!(ErrorCode::RuntimeError.as_str(), "RUNTIME_ERROR");
        assert_eq!(ErrorCode::Canceled.as_str(), "CANCELED");
    }

    #[test]
    fn test_http_status_mapping() {
        assert_eq!(ErrorCode::BoxNotFound.http_status(), 404);
        assert_eq!(ErrorCode::ExecNotFound.http_status(), 404);
        assert_eq!(ErrorCode::ResourceExceeded.http_status(), 409);
        assert_eq!(ErrorCode::NameAlreadyExists.http_status(), 409);
        assert_eq!(ErrorCode::BoxAlreadyRunning.http_status(), 409);
        assert_eq!(ErrorCode::BoxNotRunning.http_status(), 400);
        assert_eq!(ErrorCode::ExecAlreadyFinished.http_status(), 400);
        assert_eq!(ErrorCode::ImagePullFailed.http_status(), 400);
        assert_eq!(ErrorCode::Timeout.http_status(), 400);
        assert_eq!(ErrorCode::RuntimeError.http_status(), 400);
        assert_eq!(ErrorCode::Canceled.http_status(), 400);
    }

    #[test]
    fn test_error_display() {
        let err = BoxRunError::new(ErrorCode::BoxNotFound, "Box 'test' not found");
        assert_eq!(err.to_string(), "BOX_NOT_FOUND: Box 'test' not found");
        assert_eq!(err.http_status(), 404);
    }
}
