//! Writing the config on a background thread.
//!
//! [`Config::save`] flushes the file to disk before it replaces the old one, which can take a
//! noticeable moment on a slow disk, and the window saves on every change of a setting. Doing
//! that on the UI thread makes the window stutter, so the window hands the text over to a writer
//! thread instead. Changes that arrive while a write is going on are merged: only the newest
//! state is written next.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use crate::{Config, StoreError, write_atomic};

/// What the writer thread has been asked to do.
#[derive(Debug, Default)]
struct Queue {
    /// The newest config text and where it goes; older ones that were not written yet are gone.
    pending: Option<(PathBuf, String)>,
    /// The writer is being shut down: write what is pending, then end.
    stop: bool,
}

type Shared = (Mutex<Queue>, Condvar);

/// Writes the config files handed to [`ConfigWriter::submit`] in the background. Dropping it
/// writes what is still pending and waits for that to finish, so nothing is lost when the program
/// ends.
#[derive(Debug)]
pub struct ConfigWriter {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl ConfigWriter {
    /// Starts the writer thread. `on_error` is called on that thread when a file could not be
    /// written.
    #[must_use]
    pub fn start(on_error: impl Fn(StoreError) + Send + 'static) -> Self {
        let shared: Arc<Shared> = Arc::default();
        let thread = thread::Builder::new()
            .name("config-writer".to_owned())
            .spawn({
                let shared = Arc::clone(&shared);
                move || run(&shared, &on_error)
            })
            .ok();
        Self { shared, thread }
    }

    /// Queues `config` to be written to `path`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Serialize`] if the config can't be turned into text. Errors of the
    /// write itself go to the handler given to [`ConfigWriter::start`].
    pub fn submit(&self, config: &Config, path: &Path) -> Result<(), StoreError> {
        let text = config.to_toml()?;
        if self.thread.is_none() {
            // No thread could be started: write right here rather than not at all.
            return write_atomic(path, &text);
        }
        let (queue, wake) = &*self.shared;
        queue.lock().unwrap_or_else(PoisonError::into_inner).pending =
            Some((path.to_owned(), text));
        wake.notify_one();
        Ok(())
    }
}

impl Drop for ConfigWriter {
    fn drop(&mut self) {
        let (queue, wake) = &*self.shared;
        queue.lock().unwrap_or_else(PoisonError::into_inner).stop = true;
        wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(shared: &Shared, on_error: &impl Fn(StoreError)) {
    let (queue, wake) = shared;
    loop {
        let (path, text) = {
            let mut queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
            loop {
                if let Some(job) = queue.pending.take() {
                    break job;
                }
                if queue.stop {
                    return;
                }
                queue = wake.wait(queue).unwrap_or_else(PoisonError::into_inner);
            }
        };
        if let Err(error) = write_atomic(&path, &text) {
            on_error(error);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("breakbar-writer-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn config_with_path(path: &str) -> Config {
        Config {
            gw2_path: Some(path.into()),
            ..Config::default()
        }
    }

    #[test]
    fn the_newest_config_is_what_ends_up_on_disk() {
        let dir = temp_dir("newest");
        let path = dir.join("config.toml");
        let writer = ConfigWriter::start(|error| panic!("unexpected: {error}"));

        for n in 0..50 {
            writer
                .submit(&config_with_path(&format!(r"C:\Game{n}\Gw2-64.exe")), &path)
                .unwrap();
        }
        drop(writer); // waits for what is pending

        let saved = Config::load(&path).unwrap();
        assert_eq!(saved.gw2_path, Some(PathBuf::from(r"C:\Game49\Gw2-64.exe")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_is_reported_to_the_handler() {
        let dir = temp_dir("failing");
        std::fs::create_dir_all(&dir).unwrap();
        // A file where the folder of the config should be: the write can't succeed.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, b"x").unwrap();
        let (errors, received) = mpsc::channel();
        let writer = ConfigWriter::start(move |error| {
            let _ = errors.send(error.to_string());
        });

        writer
            .submit(&Config::default(), &blocker.join("config.toml"))
            .unwrap();
        drop(writer);

        assert!(received.recv().unwrap().contains("config.toml"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
