//! Sets the filter graph on running outputs over PipeWire's own protocol;
//! libpulse can't reach a node's params.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use pipewire as pw;
use pw::node::Node;
use pw::permissions::PermissionFlags;
use pw::registry::{GlobalObject, RegistryRc};
use pw::spa;
use pw::spa::pod::{Object, Property, PropertyFlags, Value};
use pw::types::ObjectType;

enum Message {
    /// Node name and graph; an empty graph removes the EQ.
    Set(String, String),
    Quit,
}

/// A connection to PipeWire on its own thread, which owns its main loop.
pub struct Live {
    sender: pw::channel::Sender<Message>,
    thread: Option<JoinHandle<()>>,
}

impl Live {
    /// Connects to PipeWire, or `None` if that fails (no socket, or no
    /// permission), so the equalizer isn't offered when it can't apply.
    pub fn start() -> Option<Self> {
        let (sender, receiver) = pw::channel::channel();
        let (ready, connected) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("openhub-eq".into())
            .spawn(move || {
                if let Err(error) = run(receiver, ready) {
                    eprintln!("Equalizer: PipeWire connection failed: {error}");
                }
            })
            .ok()?;
        let mut live = Self {
            sender,
            thread: Some(thread),
        };
        // A failed connection drops `ready`, which ends the wait at once; a
        // local socket answers well within the timeout.
        match connected.recv_timeout(Duration::from_secs(2)) {
            Ok(()) => Some(live),
            // The thread has ended; dropping `live` joins it.
            Err(mpsc::RecvTimeoutError::Disconnected) => None,
            // Still connecting: let it go rather than block the main thread
            // on a join. Drop then only asks it to quit.
            Err(mpsc::RecvTimeoutError::Timeout) => {
                live.thread.take();
                None
            }
        }
    }

    /// Sets `graph` on the output named `node`, now and whenever it reappears
    /// while Openhub runs.
    pub fn set(&self, node: &str, graph: String) {
        let _ = self.sender.send(Message::Set(node.to_owned(), graph));
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What binding a node needs, kept from its registry announcement.
#[derive(Clone, Copy)]
struct Global {
    id: u32,
    version: u32,
    permissions: PermissionFlags,
}

#[derive(Default)]
struct State {
    nodes: HashMap<String, Global>,
    /// Graphs set this session, by node name.
    wanted: HashMap<String, String>,
}

fn run(
    receiver: pw::channel::Receiver<Message>,
    ready: mpsc::SyncSender<()>,
) -> Result<(), pw::Error> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    // WirePlumber gives sandboxed clients read-only access to other nodes
    // unless they connect as a manager.
    let core = context.connect_rc(Some(pw::properties::properties! {
        "media.category" => "Manager",
        *pw::keys::APP_NAME => "Openhub",
    }))?;
    let registry = core.get_registry_rc()?;
    let state = Rc::new(RefCell::new(State::default()));

    let _registry_listener = registry
        .add_listener_local()
        .global({
            let state = state.clone();
            let registry = registry.downgrade();
            move |object| {
                if object.type_ != ObjectType::Node {
                    return;
                }
                let Some(name) = object.props.and_then(|props| props.get("node.name")) else {
                    return;
                };
                let global = Global {
                    id: object.id,
                    version: object.version,
                    permissions: object.permissions,
                };
                let mut state = state.borrow_mut();
                state.nodes.insert(name.to_owned(), global);
                if let (Some(graph), Some(registry)) = (state.wanted.get(name), registry.upgrade())
                {
                    apply(&registry, global, graph);
                }
            }
        })
        .global_remove({
            let state = state.clone();
            move |id| state.borrow_mut().nodes.retain(|_, global| global.id != id)
        })
        .register();

    let _receiver = receiver.attach(main_loop.loop_(), {
        let main_loop = main_loop.downgrade();
        move |message| match message {
            Message::Set(name, graph) => {
                let mut state = state.borrow_mut();
                if let Some(&global) = state.nodes.get(&name) {
                    apply(&registry, global, &graph);
                }
                state.wanted.insert(name, graph);
            }
            Message::Quit => {
                if let Some(main_loop) = main_loop.upgrade() {
                    main_loop.quit();
                }
            }
        }
    });

    // Connected: from here on, set() reaches PipeWire.
    let _ = ready.send(());
    main_loop.run();
    Ok(())
}

/// Sends `audioconvert.filter-graph.0` to the node, like
/// `pw-cli set-param <id> Props '{ params = [ key value ] }'`.
fn apply(registry: &RegistryRc, global: Global, graph: &str) {
    let object = GlobalObject::<&spa::utils::dict::DictRef> {
        id: global.id,
        permissions: global.permissions,
        type_: ObjectType::Node,
        version: global.version,
        props: None,
    };
    let Ok(node) = registry.bind::<Node, _>(&object) else {
        return;
    };
    let props = Value::Object(Object {
        type_: spa::sys::SPA_TYPE_OBJECT_Props,
        id: spa::sys::SPA_PARAM_Props,
        properties: vec![Property {
            key: spa::sys::SPA_PROP_params,
            flags: PropertyFlags::empty(),
            value: Value::Struct(vec![
                Value::String("audioconvert.filter-graph.0".into()),
                Value::String(graph.into()),
            ]),
        }],
    });
    let Ok((bytes, _)) =
        spa::pod::serialize::PodSerializer::serialize(std::io::Cursor::new(Vec::new()), &props)
    else {
        return;
    };
    let bytes = bytes.into_inner();
    if let Some(pod) = spa::pod::Pod::from_bytes(&bytes) {
        node.set_param(spa::param::ParamType::Props, 0, pod);
    }
}
