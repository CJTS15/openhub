//! The `org.bluez.Agent1` object BlueZ calls during pairing that Openhub
//! starts. zbus runs these methods on its own thread, so the agent only
//! forwards each request to the main loop and waits for the answer.

use futures_channel::{mpsc, oneshot};
use zbus::interface;
use zbus::zvariant::OwnedObjectPath;

pub const PATH: &str = "/io/github/cjts15/Openhub/Agent";
/// Openhub can show codes and take typed ones.
pub const CAPABILITY: &str = "KeyboardDisplay";

/// A question or notice for the user. `D` identifies the device: an object
/// path from BlueZ, then a `Device` once the manager has looked it up.
/// Dropping a `reply` sender without answering rejects the request.
pub enum PairingRequest<D> {
    /// Ask whether `passkey` matches the code the device shows.
    Confirm {
        device: D,
        passkey: u32,
        reply: oneshot::Sender<bool>,
    },
    /// Ask for the PIN code to type for a legacy device.
    PinCode {
        device: D,
        reply: oneshot::Sender<Option<String>>,
    },
    /// Ask for the six-digit passkey the device shows.
    Passkey {
        device: D,
        reply: oneshot::Sender<Option<u32>>,
    },
    /// Show `code` for the user to type on the device. Sent again as they type.
    Display { device: D, code: String },
    /// BlueZ gave up on the current request.
    Cancel,
}

impl<D> PairingRequest<D> {
    /// Swaps the device identifier, or drops the request (rejecting it) when
    /// `f` can't find the device.
    pub fn map_device<E>(self, f: impl FnOnce(D) -> Option<E>) -> Option<PairingRequest<E>> {
        Some(match self {
            Self::Confirm {
                device,
                passkey,
                reply,
            } => PairingRequest::Confirm {
                device: f(device)?,
                passkey,
                reply,
            },
            Self::PinCode { device, reply } => PairingRequest::PinCode {
                device: f(device)?,
                reply,
            },
            Self::Passkey { device, reply } => PairingRequest::Passkey {
                device: f(device)?,
                reply,
            },
            Self::Display { device, code } => PairingRequest::Display {
                device: f(device)?,
                code,
            },
            Self::Cancel => PairingRequest::Cancel,
        })
    }
}

#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
}

fn rejected() -> AgentError {
    AgentError::Rejected("Rejected by the user".into())
}

pub struct Agent {
    pub requests: mpsc::UnboundedSender<PairingRequest<OwnedObjectPath>>,
}

impl Agent {
    async fn ask<T>(
        &self,
        request: impl FnOnce(oneshot::Sender<T>) -> PairingRequest<OwnedObjectPath>,
    ) -> Option<T> {
        let (reply, answer) = oneshot::channel();
        self.requests.unbounded_send(request(reply)).ok()?;
        answer.await.ok()
    }
}

#[interface(name = "org.bluez.Agent1")]
impl Agent {
    fn release(&self) {}

    async fn request_pin_code(&self, device: OwnedObjectPath) -> Result<String, AgentError> {
        self.ask(|reply| PairingRequest::PinCode { device, reply })
            .await
            .flatten()
            .ok_or_else(rejected)
    }

    fn display_pin_code(&self, device: OwnedObjectPath, pincode: String) {
        let _ = self.requests.unbounded_send(PairingRequest::Display {
            device,
            code: pincode,
        });
    }

    async fn request_passkey(&self, device: OwnedObjectPath) -> Result<u32, AgentError> {
        self.ask(|reply| PairingRequest::Passkey { device, reply })
            .await
            .flatten()
            .ok_or_else(rejected)
    }

    fn display_passkey(&self, device: OwnedObjectPath, passkey: u32, _entered: u16) {
        let _ = self.requests.unbounded_send(PairingRequest::Display {
            device,
            code: format!("{passkey:06}"),
        });
    }

    async fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
    ) -> Result<(), AgentError> {
        let confirmed = self
            .ask(|reply| PairingRequest::Confirm {
                device,
                passkey,
                reply,
            })
            .await;
        if confirmed == Some(true) {
            Ok(())
        } else {
            Err(rejected())
        }
    }

    /// Only the default agent gets incoming pairing and service requests,
    /// and Openhub doesn't ask to be the default, so refuse anything that
    /// still arrives.
    fn request_authorization(&self, _device: OwnedObjectPath) -> Result<(), AgentError> {
        Err(rejected())
    }

    fn authorize_service(&self, _device: OwnedObjectPath, _uuid: String) -> Result<(), AgentError> {
        Err(rejected())
    }

    fn cancel(&self) {
        let _ = self.requests.unbounded_send(PairingRequest::Cancel);
    }
}
