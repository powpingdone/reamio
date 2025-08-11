use std::{collections::HashMap, sync::Weak};

use tokio::sync::{RwLock, watch};

#[allow(unused)]
pub use tracing::{Instrument, Level};
#[allow(unused)]
pub use tracing::{debug, error, event, info, trace, warn};
#[allow(unused)]
pub use tracing::{debug_span, error_span, span, info_span, trace_span, warn_span};

pub use sqlx::SqlitePool;
pub use sqlx::prelude::*;
pub use tracing_subscriber::prelude::*;

pub use crate::error::*;
pub use crate::fetch_users_music_db;

pub type MusicDbMapRef = Weak<RwLock<HashMap<String, SqlitePool>>>;
pub type WakeTx<T> = watch::Sender<T>;
pub type WakeRx<T> = watch::Receiver<T>;

// zero sized types for wakeup
pub struct PopulateMetadata;
