//! Protobuf v1 contracts and generated types for the Wavemaker crates.
//!
//! Packages follow docs/spec/protobuf-conventions.md (WM-0013):
//! `wavemaker.{common,control,backend,engine,metrics}.v1`. Codegen lands in
//! WM-0102.

/// Wire protocol version carried by every handshake/registration.
///
/// Incompatible versions must be rejected at handshake time — never
/// silently downgraded (protobuf-conventions §2.3).
pub const PROTOCOL_VERSION: u32 = 1;

#[cfg(test)]
mod tests {
    #[test]
    fn protocol_version_is_frozen_at_one() {
        assert_eq!(super::PROTOCOL_VERSION, 1);
    }
}
