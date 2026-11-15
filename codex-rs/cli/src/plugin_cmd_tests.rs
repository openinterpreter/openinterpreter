//! Verify plugin-list output error handling at the writer boundary.

use super::ListPluginsArgs;
use super::write_plugin_list;
use pretty_assertions::assert_eq;
use std::io;
use std::io::Write;

#[derive(Clone, Copy)]
enum Failure {
    Write,
    Flush,
}

struct FailingWriter {
    failure: Failure,
    kind: io::ErrorKind,
}

impl Write for FailingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self.failure {
            Failure::Write => Err(io::Error::new(self.kind, "output failed")),
            Failure::Flush => Ok(bytes.len()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.failure {
            Failure::Write => Ok(()),
            Failure::Flush => Err(io::Error::new(self.kind, "output failed")),
        }
    }
}

#[test]
fn plugin_list_treats_broken_pipe_as_success() {
    for json in [false, true] {
        for failure in [Failure::Write, Failure::Flush] {
            let mut writer = FailingWriter {
                failure,
                kind: io::ErrorKind::BrokenPipe,
            };
            let args = ListPluginsArgs {
                marketplace_name: None,
                json,
                available: true,
            };

            write_plugin_list(&mut writer, Vec::new(), args).unwrap();
        }
    }
}

#[test]
fn plugin_list_preserves_other_output_errors() {
    for json in [false, true] {
        for failure in [Failure::Write, Failure::Flush] {
            for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::WriteZero] {
                let mut writer = FailingWriter { failure, kind };
                let args = ListPluginsArgs {
                    marketplace_name: None,
                    json,
                    available: true,
                };

                let error = write_plugin_list(&mut writer, Vec::new(), args)
                    .unwrap_err()
                    .downcast::<io::Error>()
                    .unwrap();
                assert_eq!(error.kind(), kind);
                assert_eq!(error.to_string(), "output failed");
            }
        }
    }
}
