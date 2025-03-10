use std::{array::TryFromSliceError, fmt, io, num::TryFromIntError, path::PathBuf};

use veilid_core::{VeilidAPIError, VeilidStateAttachment};

use crate::proto;

#[derive(Debug)]
pub enum Error {
    /// Unexpected fault.
    Fault(Unexpected),
    /// Error related to file indexing.
    Index {
        path: Option<PathBuf>,
        err: stigmerge_fileindex::Error,
    },
    /// Error caused by local filesystem operation.
    LocalFile(io::Error),
    /// Error caused by Veilid network operation.
    Node {
        state: NodeState,
        err: VeilidAPIError,
    },
    /// Protocol error when decoding a message from a remote peer.
    /// Similar to an HTTP 400, "it's not me it's you."
    RemoteProtocol(proto::Error),
    /// Protocol error when trying to encode a message.
    /// Similar to an HTTP 500, "it's not you it's me."
    InternalProtocol(proto::Error),
    ResetTimeout,
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Fault(u) => write!(f, "unexpected fault: {}", u),
            Error::Index { path, err } => write!(
                f,
                "failed to index{}: {}",
                match path {
                    Some(p) => format!(" {:?}", p),
                    None => "".to_string(),
                },
                err
            ),
            Error::LocalFile(err) => {
                write!(f, "file operation failed: {}", err)
            }
            Error::Node { state, err } => write!(f, "{}: {}", state, err),
            Error::RemoteProtocol(e) => write!(f, "invalid response from remote peer: {}", e),
            Error::InternalProtocol(e) => write!(f, "failed to encode protocol message: {}", e),
            Error::ResetTimeout => write!(f, "reset timeout"),
        }
    }
}

impl From<VeilidAPIError> for Error {
    fn from(err: VeilidAPIError) -> Self {
        match err {
            VeilidAPIError::NotInitialized => Error::Node {
                state: NodeState::APINotStarted,
                err,
            },
            VeilidAPIError::AlreadyInitialized => Error::Fault(Unexpected::Veilid(err)),
            VeilidAPIError::Timeout => Error::Node {
                state: NodeState::RemotePeerNotAvailable,
                err,
            },
            VeilidAPIError::TryAgain { message: _ } => Error::Node {
                state: NodeState::RemotePeerNotAvailable,
                err,
            },
            VeilidAPIError::Shutdown => Error::Node {
                state: NodeState::APIShuttingDown,
                err,
            },
            VeilidAPIError::InvalidTarget { message: _ } => Error::Node {
                state: NodeState::PrivateRouteInvalid,
                err,
            },
            VeilidAPIError::NoConnection { message: _ } => Error::Node {
                state: NodeState::NetworkNotAvailable,
                err,
            },
            VeilidAPIError::KeyNotFound { key: _ } => Error::Node {
                state: NodeState::RemotePeerNotAvailable,
                err,
            },
            VeilidAPIError::Internal { message: _ } => Error::Fault(Unexpected::Veilid(err)),
            VeilidAPIError::Unimplemented { message: _ } => Error::Fault(Unexpected::Veilid(err)),
            VeilidAPIError::ParseError {
                message: _,
                value: _,
            } => Error::Fault(Unexpected::Veilid(err)),
            VeilidAPIError::InvalidArgument {
                context: _,
                argument: _,
                value: _,
            } => Error::Fault(Unexpected::Veilid(err)),
            VeilidAPIError::MissingArgument {
                context: _,
                argument: _,
            } => Error::Fault(Unexpected::Veilid(err)),
            VeilidAPIError::Generic { message: _ } => Error::Fault(Unexpected::Veilid(err)),
        }
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(err: std::str::Utf8Error) -> Self {
        Error::Fault(Unexpected::Utf8(err))
    }
}
impl From<TryFromIntError> for Error {
    fn from(err: TryFromIntError) -> Self {
        Error::Fault(Unexpected::IntOverflow(err))
    }
}
impl From<TryFromSliceError> for Error {
    fn from(err: TryFromSliceError) -> Self {
        Error::Fault(Unexpected::SliceSize(err))
    }
}
impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::LocalFile(err)
    }
}
impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Error::other(err)
    }
}

impl Error {
    pub fn msg<T: fmt::Display + fmt::Debug + Send + Sync + 'static>(s: T) -> Error {
        Error::Fault(Unexpected::Other(anyhow::Error::msg(s)))
    }
    pub fn other<E: Into<anyhow::Error>>(err: E) -> Error {
        Error::Fault(Unexpected::Other(err.into()))
    }
    pub fn index(err: stigmerge_fileindex::Error) -> Error {
        Error::Index { path: None, err }
    }
    pub fn remote_protocol(err: proto::Error) -> Error {
        Error::RemoteProtocol(err)
    }
    pub fn internal_protocol(err: proto::Error) -> Error {
        Error::InternalProtocol(err)
    }
    pub fn cancelled<E>(_err: E) -> Error {
        Error::Fault(Unexpected::Cancelled)
    }

    pub fn is_route_invalid(&self) -> bool {
        if let Error::Node { state, err: _ } = self {
            *state == NodeState::PrivateRouteInvalid
        } else {
            false
        }
    }

    pub fn is_retriable(&self) -> bool {
        match self {
            Error::Fault(_) => true,
            Error::Node { state, err: _ } => *state == NodeState::RemotePeerNotAvailable,
            Error::RemoteProtocol(_) => true,
            Error::ResetTimeout => true,
            _ => false,
        }
    }

    pub fn is_resetable(&self) -> bool {
        match self {
            Error::Index { path: _, err: _ } => false,
            Error::LocalFile(_) => false,
            Error::InternalProtocol(_) => false,
            Error::Node { state, err: _ } => match state {
                NodeState::APIShuttingDown => false,
                NodeState::PrivateRouteInvalid => false,
                _ => true,
            },
            Error::Fault(err) => match err {
                Unexpected::Veilid(VeilidAPIError::Unimplemented { message: _ }) => false,
                Unexpected::Veilid(VeilidAPIError::Shutdown) => false,
                Unexpected::Veilid(_) => true,
                _ => false,
            },
            _ => true,
        }
    }

    pub fn is_shutdown(&self) -> bool {
        if let Error::Fault(Unexpected::Veilid(VeilidAPIError::Shutdown)) = self {
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum NodeState {
    APINotStarted,
    NetworkNotAvailable,
    RemotePeerNotAvailable,
    PrivateRouteInvalid,
    Connecting,
    Connected,
    APIShuttingDown,
}

impl NodeState {
    pub fn is_connected(&self) -> bool {
        *self == NodeState::Connected
    }
}

impl From<&Box<VeilidStateAttachment>> for NodeState {
    fn from(attachment: &Box<VeilidStateAttachment>) -> NodeState {
        let is_attach = match attachment.state {
            veilid_core::AttachmentState::Detached => false,
            veilid_core::AttachmentState::Attaching => true,
            veilid_core::AttachmentState::AttachedWeak => true,
            veilid_core::AttachmentState::AttachedGood => true,
            veilid_core::AttachmentState::AttachedStrong => true,
            veilid_core::AttachmentState::FullyAttached => true,
            veilid_core::AttachmentState::OverAttached => true,
            _ => false,
        };
        if attachment.public_internet_ready {
            NodeState::Connected
        } else if is_attach {
            NodeState::Connecting
        } else {
            NodeState::NetworkNotAvailable
        }
    }
}

impl fmt::Display for NodeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeState::APINotStarted => write!(f, "api not started"),
            NodeState::NetworkNotAvailable => write!(f, "network not available"),
            NodeState::RemotePeerNotAvailable => write!(f, "remote peer not available"),
            NodeState::PrivateRouteInvalid => write!(f, "private route invalid"),
            NodeState::APIShuttingDown => write!(f, "api shutdown in progress"),
            NodeState::Connecting => write!(f, "connecting to network"),
            NodeState::Connected => write!(f, "connected"),
        }
    }
}

#[derive(Debug)]
pub enum Unexpected {
    Veilid(VeilidAPIError),
    Utf8(std::str::Utf8Error),
    IntOverflow(TryFromIntError),
    SliceSize(TryFromSliceError),
    Cancelled,
    Other(anyhow::Error),
}

impl fmt::Display for Unexpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unexpected::Veilid(e) => write!(f, "veilid: {}", e),
            Unexpected::Utf8(e) => write!(f, "utf8 encoding failed: {}", e),
            Unexpected::IntOverflow(e) => write!(f, "integer overflow: {}", e),
            Unexpected::SliceSize(e) => write!(f, "unexpected slice size: {}", e),
            Unexpected::Cancelled => write!(f, "cancelled"),
            Unexpected::Other(e) => write!(f, "{}", e),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
