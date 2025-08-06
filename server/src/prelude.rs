use std::{collections::HashMap, sync::Weak};

use tokio::sync::{watch, RwLock};

#[allow(unused)]
pub use tracing::{debug, error, info, trace, warn, event, Level};

pub use tracing_subscriber::prelude::*;
pub use sqlx::prelude::*;
pub use sqlx::SqlitePool;

pub use crate::error::*;
pub use crate::fetch_users_music_db;

pub type MusicDbMapRef = Weak<RwLock<HashMap<String, SqlitePool>>>;
pub type WakeTx<T> = watch::Sender<T>;
pub type WakeRx<T> = watch::Receiver<T>;

// zero sized types for wakeup
pub struct PopulateMetadata;
