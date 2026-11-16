//! One in-flight AT response, with quarantine after timeout.
//! A late terminal response must be consumed before another command may start.

#[derive(Debug)]
pub struct ResponseSlot<T> {
    pending: Option<T>,
    draining: bool,
}

impl<T> Default for ResponseSlot<T> {
    fn default() -> Self {
        Self {
            pending: None,
            draining: false,
        }
    }
}

impl<T> ResponseSlot<T> {
    pub fn begin(&mut self, value: T) -> Result<(), &'static str> {
        if self.draining {
            return Err(
                "上条指令已超时，正在等待其终结响应；请稍后重试，持续无响应时复位设备后重连",
            );
        }
        if self.pending.is_some() {
            return Err("已有指令正在等待响应");
        }
        self.pending = Some(value);
        Ok(())
    }

    pub fn pending_mut(&mut self) -> Option<&mut T> {
        self.pending.as_mut()
    }

    pub fn is_draining(&self) -> bool {
        self.draining
    }

    /// Also consumes a late final without attributing it to a new command.
    pub fn finish(&mut self) -> Option<T> {
        self.draining = false;
        self.pending.take()
    }

    /// Called under the same lock as finish, so a completed reply cannot
    /// accidentally quarantine an already-idle channel.
    pub fn timeout(&mut self) -> Option<T> {
        let pending = self.pending.take();
        if pending.is_some() {
            self.draining = true;
        }
        pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_final_cannot_complete_next_command() {
        let mut slot = ResponseSlot::default();
        slot.begin("first").unwrap();
        assert_eq!(slot.timeout(), Some("first"));
        assert!(slot.begin("second").is_err());
        assert_eq!(slot.finish(), None); // first command's late OK
        slot.begin("second").unwrap();
        assert_eq!(slot.finish(), Some("second"));
    }

    #[test]
    fn completion_winning_timeout_race_does_not_quarantine() {
        let mut slot = ResponseSlot::default();
        slot.begin(1).unwrap();
        assert_eq!(slot.finish(), Some(1));
        assert_eq!(slot.timeout(), None);
        assert!(slot.begin(2).is_ok());
    }

    #[test]
    fn pending_command_cannot_be_overwritten() {
        let mut slot = ResponseSlot::default();
        slot.begin(1).unwrap();
        assert!(slot.begin(2).is_err());
        assert_eq!(slot.finish(), Some(1));
    }
}
