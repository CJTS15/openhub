use std::cell::RefCell;

use adw::prelude::*;
use futures_channel::oneshot;

use crate::bluez::PairingRequest;
use crate::model::Device;

/// Builds the dialog that answers `request`, or `None` for `Cancel`. A
/// `Display` dialog only has a "cancel" response, which the caller handles.
pub fn new(request: PairingRequest<Device>) -> Option<adw::AlertDialog> {
    Some(match request {
        PairingRequest::Confirm {
            device,
            passkey,
            reply,
        } => {
            let body = format!("Check that {} shows the code {passkey:06}.", device.name());
            let dialog = alert("Confirm Pairing", &body, Some("_Pair"));
            answer_with(&dialog, reply, |accepted| accepted);
            dialog
        }
        PairingRequest::PinCode { device, reply } => {
            let body = format!(
                "Enter the PIN code for {}. Many devices use 0000.",
                device.name()
            );
            let dialog = alert("Enter PIN Code", &body, Some("_Pair"));
            let entry = gtk::Entry::builder().activates_default(true).build();
            dialog.set_extra_child(Some(&entry));
            answer_with(&dialog, reply, move |accepted| {
                let pin = entry.text();
                (accepted && !pin.is_empty()).then(|| pin.to_string())
            });
            dialog
        }
        PairingRequest::Passkey { device, reply } => {
            let body = format!("Enter the six-digit code {} shows.", device.name());
            let dialog = alert("Enter Passkey", &body, Some("_Pair"));
            let entry = gtk::Entry::builder()
                .activates_default(true)
                .input_purpose(gtk::InputPurpose::Digits)
                .max_length(6)
                .build();
            dialog.set_extra_child(Some(&entry));
            answer_with(&dialog, reply, move |accepted| {
                accepted.then(|| entry.text().parse().ok()).flatten()
            });
            dialog
        }
        PairingRequest::Display { device, code } => {
            alert("Pairing", &display_body(&device, &code), None)
        }
        PairingRequest::Cancel => return None,
    })
}

pub fn display_body(device: &Device, code: &str) -> String {
    format!("Type {code} on {}, then press Enter.", device.name())
}

fn alert(heading: &str, body: &str, accept: Option<&str>) -> adw::AlertDialog {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("cancel", "_Cancel");
    dialog.set_close_response("cancel");
    if let Some(label) = accept {
        dialog.add_response("accept", label);
        dialog.set_response_appearance("accept", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("accept"));
    }
    dialog
}

/// Sends `answer(accepted)` on the first response. If the dialog goes away
/// without one, dropping `reply` rejects the request.
fn answer_with<T: 'static>(
    dialog: &adw::AlertDialog,
    reply: oneshot::Sender<T>,
    answer: impl Fn(bool) -> T + 'static,
) {
    let reply = RefCell::new(Some(reply));
    dialog.connect_response(None, move |_, response| {
        if let Some(reply) = reply.take() {
            let _ = reply.send(answer(response == "accept"));
        }
    });
}
