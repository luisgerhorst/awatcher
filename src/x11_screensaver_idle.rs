use std::{sync::Arc, thread};

use chrono::{Duration, Utc};

use crate::{report_client::ReportClient, x11_connection::X11Connection, BoxedError, Watcher};

pub struct IdleWatcher {
    connection: X11Connection,
}

impl IdleWatcher {
    fn run(&self, is_idle: bool, client: &Arc<ReportClient>) -> Result<bool, BoxedError> {
        // The logic is rewritten from the original Python code:
        // https://github.com/ActivityWatch/aw-watcher-afk/blob/ef531605cd8238e00138bbb980e5457054e05248/aw_watcher_afk/afk.py#L73
        let duration_1ms: Duration = Duration::milliseconds(1);
        let duration_zero: Duration = Duration::zero();

        let seconds_since_input = self.connection.seconds_since_last_input()?;
        let now = Utc::now();
        let time_since_input = Duration::seconds(i64::from(seconds_since_input));
        let last_input = now - time_since_input;
        let mut is_idle_again = is_idle;

        if is_idle && u64::from(seconds_since_input) < client.config.idle_timeout.as_secs() {
            debug!("No longer idle");
            client.ping(is_idle, last_input, duration_zero)?;
            is_idle_again = false;
            // ping with timestamp+1ms with the next event (to ensure the latest event gets retrieved by get_event)
            client.ping(is_idle, last_input + duration_1ms, duration_zero)?;
        } else if !is_idle && u64::from(seconds_since_input) >= client.config.idle_timeout.as_secs()
        {
            debug!("Idle again");
            client.ping(is_idle, last_input, duration_zero)?;
            is_idle_again = true;
            // ping with timestamp+1ms with the next event (to ensure the latest event gets retrieved by get_event)
            client.ping(is_idle, last_input + duration_1ms, time_since_input)?;
        } else {
            // Send a heartbeat if no state change was made
            if is_idle {
                trace!("Reporting as idle");
                client.ping(is_idle, last_input, time_since_input)?;
            } else {
                trace!("Reporting as not idle");
                client.ping(is_idle, last_input, duration_zero)?;
            }
        }

        Ok(is_idle_again)
    }
}

impl Watcher for IdleWatcher {
    fn new() -> Result<Self, BoxedError> {
        let connection = X11Connection::new()?;

        // Check if screensaver extension is supported
        connection.seconds_since_last_input()?;

        Ok(IdleWatcher { connection })
    }

    fn watch(&mut self, client: &Arc<ReportClient>) {
        info!("Starting idle watcher");
        let mut is_idle = false;
        loop {
            match self.run(is_idle, client) {
                Ok(is_idle_again) => {
                    is_idle = is_idle_again;
                }
                Err(e) => error!("Error on idle iteration: {e}"),
            };

            thread::sleep(client.config.poll_time_idle);
        }
    }
}