//! What cloofy shares between music services: the data model the interface shows, where files
//! go, settings, updates, and what players report. Nothing in here knows Spotify's protocols,
//! so a build without Spotify leaves librespot out entirely.

pub mod artist;
pub mod catalog;
pub mod config;
pub mod connect;
pub mod home;
pub mod http;
pub mod lyrics;
pub mod paths;
pub mod player;
pub mod service;
pub mod update;
