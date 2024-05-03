use std::{array::TryFromSliceError, fmt, io, num::TryFromIntError, path::PathBuf};

use veilid_core::VeilidAPIError;

use crate::proto;

#[derive(Debug)]
pub enum Error {
    /// Unexpected fault.
    Fault(Unexpected),
    /// Error related to file indexing.
    Index {
        path: Option<PathBuf>,
        err: distrans_fileindex::Error,
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

impl Error {
    pub fn other<S: ToString>(e: S) -> Error {
        Error::Fault(Unexpected::Other(e.to_string()))
    }
    pub fn index(err: distrans_fileindex::Error) -> Error {
        Error::Index { path: None, err }
    }
    pub fn remote_protocol(err: proto::Error) -> Error {
        Error::RemoteProtocol(err)
    }
    pub fn internal_protocol(err: proto::Error) -> Error {
        Error::InternalProtocol(err)
    }

    pub fn is_route_invalid(err: &Error) -> bool {
        if let Error::Node { state, err: _ } = err {
            *state == NodeState::PrivateRouteInvalid
        } else {
            false
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
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
    pub fn is_connected(node_state: &Self) -> bool {
        *node_state == NodeState::Connected
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
    Other(String),
}

impl fmt::Display for Unexpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Unexpected::Veilid(e) => write!(f, "veilid: {}", e),
            Unexpected::Utf8(e) => write!(f, "utf8 encoding failed: {}", e),
            Unexpected::IntOverflow(e) => write!(f, "integer overflow: {}", e),
            Unexpected::SliceSize(e) => write!(f, "unexpected slice size: {}", e),
            Unexpected::Other(e) => write!(f, "{}", e),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
