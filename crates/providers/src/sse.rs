use myagent_core::{AgentError, Result};
/// Decodifica bytes antes de UTF-8: suporta CRLF e caracteres fragmentados.
pub struct SseDecoder {
    pending: Vec<u8>,
    data: Vec<u8>,
    event: String,
    total: usize,
    after_cr: bool,
    first_line: bool,
    pub max_frame: usize,
    pub max_total: usize,
}
#[derive(Debug)]
pub struct SseFrame {
    pub event: String,
    pub data: String,
}
impl SseDecoder {
    pub fn new(max_frame: usize, max_total: usize) -> Self {
        Self {
            pending: Vec::new(),
            data: Vec::new(),
            event: String::new(),
            total: 0,
            after_cr: false,
            first_line: true,
            max_frame,
            max_total,
        }
    }
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseFrame>> {
        self.total = self
            .total
            .checked_add(chunk.len())
            .ok_or(AgentError::Limit)?;
        if self.total > self.max_total {
            return Err(AgentError::Limit);
        }
        let mut frames = Vec::new();
        for &byte in chunk {
            if self.after_cr {
                self.after_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            if byte == b'\r' {
                self.after_cr = true;
            }
            if byte == b'\n' || byte == b'\r' {
                let mut line = std::mem::take(&mut self.pending);
                if self.first_line {
                    self.first_line = false;
                    if line.starts_with(&[0xef, 0xbb, 0xbf]) {
                        line.drain(..3);
                    }
                }
                if line.is_empty() {
                    if !self.data.is_empty() {
                        self.data.pop();
                        frames.push(SseFrame {
                            event: std::mem::take(&mut self.event),
                            data: String::from_utf8(std::mem::take(&mut self.data))
                                .map_err(|_| AgentError::Protocol)?,
                        });
                    } else {
                        self.event.clear();
                    }
                } else if !line.starts_with(b":") {
                    let (name, mut value) =
                        if let Some(index) = line.iter().position(|b| *b == b':') {
                            (&line[..index], &line[index + 1..])
                        } else {
                            (line.as_slice(), &[][..])
                        };
                    if value.first() == Some(&b' ') {
                        value = &value[1..];
                    }
                    match name {
                        b"data" => {
                            if self.data.len() + value.len() + 1 > self.max_frame {
                                return Err(AgentError::Limit);
                            }
                            self.data.extend_from_slice(value);
                            self.data.push(b'\n');
                        }
                        b"event" => {
                            self.event = String::from_utf8(value.to_vec())
                                .map_err(|_| AgentError::Protocol)?
                        }
                        _ => {}
                    }
                }
            } else {
                if self.pending.len() >= self.max_frame {
                    return Err(AgentError::Limit);
                }
                self.pending.push(byte);
            }
        }
        Ok(frames)
    }
    pub fn finish(&self) -> Result<()> {
        if !self.pending.is_empty() || !self.data.is_empty() {
            return Err(AgentError::Protocol);
        }
        Ok(())
    }
}
