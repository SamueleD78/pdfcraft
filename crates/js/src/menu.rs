use super::host::host;
#[cfg(not(target_arch = "wasm32"))]
use super::run::{initialization_limit, run_here};
#[cfg(not(target_arch = "wasm32"))]
use super::{Arc, DocInfo, Event, FieldState, Limits, SCRIPT_STACK, refuse};
use super::{Context, JsArray, JsResult, JsValue, Outcome, RefCell, Request, NativeFunction, ObjectInitializer, arg, error, function, js_string, s, text};
use boa_engine::property::PropertyDescriptor;

/// A menu tree; the UI never evaluates script-provided text.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuItem {
    pub name: String,
    pub value: String,
    pub enabled: bool,
    pub marked: bool,
    pub children: Vec<MenuItem>,
}

/// UI rendezvous. Only the script worker waits for the reply.
pub enum InteractiveEvent {
    /// Suspend the worker until a validated leaf value, cancellation or reply timeout.
    Menu(Vec<MenuItem>, std::sync::mpsc::Sender<Option<String>>),
    /// Final outcome; the host must revalidate its document before applying any effects.
    Complete(Outcome),
}
thread_local! {
    static MENU_HOST: RefCell<Option<(std::sync::mpsc::Sender<InteractiveEvent>, usize)>> = const { RefCell::new(None) };
}

/// Read one menu tree, sharing `remaining` across all arguments and descendants.
/// `popup_menu` supplies 1024 entries; roots have depth zero and depths above 16 fail.
/// Names/return values are limited to 4096 UTF-8 bytes each; absent cReturn uses cName.
/// Malformed items, exceeded limits and Boa property/conversion errors are returned.
/// Reading properties may run script code; these bounds are not a hard time/heap quota.
fn read_menu(value: &JsValue, ctx: &mut Context, depth: usize, remaining: &mut usize) -> JsResult<MenuItem> {
    if depth > 16 || *remaining == 0 {
        return Err(error("popup menu exceeds its size or depth limit"));
    }
    *remaining -= 1;
    let obj = value.as_object().ok_or_else(|| error("popup menu item must be an object"))?;
    let name = text(&obj.get(js_string!("cName"), ctx)?, ctx)?;
    let ret = obj.get(js_string!("cReturn"), ctx)?;
    let value = if ret.is_undefined() { name.clone() } else { text(&ret, ctx)? };
    if name.len() > 4096 || value.len() > 4096 {
        return Err(error("popup menu text exceeds its limit"));
    }
    let enabled = obj.get(js_string!("bEnabled"), ctx)?;
    let marked = obj.get(js_string!("bMarked"), ctx)?.to_boolean();
    let sub = obj.get(js_string!("oSubMenu"), ctx)?;
    let mut children = Vec::new();
    if let Some(o) = sub.as_object() {
        if o.is_array() {
            let array = JsArray::from_object(o.clone())?;
            let len = array.length(ctx)?;
            if len > *remaining as u64 {
                return Err(error("popup menu has too many entries"));
            }
            for i in 0..len {
                children.push(read_menu(&array.get(i, ctx)?, ctx, depth + 1, remaining)?);
            }
        } else {
            children.push(read_menu(&sub, ctx, depth + 1, remaining)?);
        }
    } else if !sub.is_undefined() && !sub.is_null() {
        return Err(error("invalid popup submenu"));
    }
    Ok(MenuItem { name, value, enabled: enabled.is_undefined() || enabled.to_boolean(), marked, children })
}

/// Parse a bounded menu and rendezvous with the interactive host, up to 32 times per action.
/// An empty menu returns null without a host request, while retaining the call budget.
/// Without a host, a successfully parsed menu returns null. Only the script worker waits
/// for a reply: up to 300 seconds, with timeout, cancellation or reply disconnection
/// yielding null. Sending to a closed host or receiving a forged selection is an error.
/// A selection must match an enabled, non-separator leaf through enabled ancestors.
/// Reply validation does not authorize document effects; the caller revalidates completion.
pub(crate) fn popup_menu(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let mut remaining = 1024;
    let mut items = Vec::new();
    for arg in args {
        items.push(read_menu(arg, ctx, 0, &mut remaining)?);
    }
    let sender = MENU_HOST.with(|h| {
        let mut host = h.borrow_mut();
        let Some((sender, count)) = host.as_mut() else { return Ok(None) };
        if *count >= 32 {
            return Err(error("too many popup menus in one action"));
        }
        *count += 1;
        Ok(Some(sender.clone()))
    })?;
    // Empty calls still count against the interaction budget, but need no reply.
    if items.is_empty() {
        return Ok(JsValue::null());
    }
    let Some(sender) = sender else { return Ok(JsValue::null()) };
    let (tx, rx) = std::sync::mpsc::channel();
    sender.send(InteractiveEvent::Menu(items.clone(), tx)).map_err(|_| error("popup host closed"))?;
    let selected = rx.recv_timeout(std::time::Duration::from_secs(300)).ok().flatten();
    fn allowed(items: &[MenuItem], value: &str) -> bool {
        items.iter().any(|i| i.enabled && i.name != "-" && (if i.children.is_empty() { i.value == value } else { allowed(&i.children, value) }))
    }
    match selected {
        Some(value) if allowed(&items, &value) => Ok(s(&value)),
        Some(_) => Err(error("invalid popup menu selection")),
        None => Ok(JsValue::null()),
    }
}

/// Return a deferred document proxy; it never opens files or bypasses consent.
/// Each proxy records its own zero-based page number on its associated request.
pub(crate) fn open_doc(_: &JsValue, args: &[JsValue], ctx: &mut Context) -> JsResult<JsValue> {
    let first = arg(args, 0);
    let path = if let Some(o) = first.as_object() {
        let fs = o.get(js_string!("cFS"), ctx)?;
        let base = o.get(js_string!("oDoc"), ctx)?;
        if (!fs.is_undefined() && !text(&fs, ctx)?.is_empty()) || (!base.is_undefined() && !base.is_null()) {
            return Err(error("openDoc supports only the current document's local file system"));
        }
        text(&o.get(js_string!("cPath"), ctx)?, ctx)?
    } else {
        // Acrobat supports app.openDoc(path, this), referring to the current document.
        let same_doc = args.get(1).is_none_or(|v| v.is_undefined() || v.is_null() || v.as_object() == Some(ctx.global_object()));
        if !same_doc || !args.iter().skip(2).all(|v| v.is_undefined() || v.is_null()) {
            return Err(error("unsupported openDoc options"));
        }
        text(&first, ctx)?
    };
    if path.is_empty() || path.len() > 4096 {
        return Err(error("invalid openDoc path"));
    }
    let index = host(ctx)?.borrow_mut().request_open_doc(path).ok_or_else(|| error("openDoc request limit reached"))?;
    fn get(_: &JsValue, _: &[JsValue], index: &usize, ctx: &mut Context) -> JsResult<JsValue> {
        let h = host(ctx)?.borrow();
        match h.requests.get(*index) {
            Some(Request::OpenDoc { page, .. }) => Ok(JsValue::from(page.unwrap_or(0) as f64)),
            _ => Err(error("openDoc handle expired")),
        }
    }
    fn set(_: &JsValue, args: &[JsValue], index: &usize, ctx: &mut Context) -> JsResult<JsValue> {
        let n = arg(args, 0).to_number(ctx)?;
        if !n.is_finite() {
            return Err(error("openDoc pageNum must be finite"));
        }
        let page = n.max(0.0) as usize;
        if !host(ctx)?.borrow_mut().open_doc_page(*index, page) {
            return Err(error("openDoc handle expired"));
        }
        Ok(JsValue::undefined())
    }
    let getter = function(ctx, NativeFunction::from_copy_closure_with_captures(get, index));
    let setter = function(ctx, NativeFunction::from_copy_closure_with_captures(set, index));
    let proxy = ObjectInitializer::new(ctx).build();
    proxy.define_property_or_throw(js_string!("pageNum"), PropertyDescriptor::builder().get(getter).set(setter).configurable(true), ctx)?;
    Ok(proxy.into())
}

/// Start a fresh script worker with the normal cooperative Boa and host limits.
/// There is no hard wall-time/heap bound or admission limit for callers of this API.
/// Dropping the receiver discards queued menus; an already received menu must be
/// cancelled through its reply sender. Abandoning results does not terminate the VM.
#[cfg(not(target_arch = "wasm32"))]
pub fn interactive(
    script: String,
    event: Event,
    doc: DocInfo,
    fields: Vec<FieldState>,
    doc_scripts: Vec<String>,
) -> Result<std::sync::mpsc::Receiver<InteractiveEvent>, String> {
    if let Some(why) =
        initialization_limit(&doc_scripts).or_else(|| std::iter::once(script.as_str()).chain(doc_scripts.iter().map(String::as_str)).find_map(refuse))
    {
        return Err(why);
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("pdfcraft-js-interactive".into())
        .stack_size(SCRIPT_STACK)
        .spawn(move || {
            MENU_HOST.with(|h| *h.borrow_mut() = Some((tx.clone(), 0)));
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let kept: Vec<_> = doc_scripts.iter().enumerate().map(|(i, d)| (i, d.as_str())).collect();
                run_here(&script, &event, &doc, Arc::new(fields), &kept, &[], Limits::default())
            }))
            .unwrap_or_else(|_| Outcome { error: Some("script worker failed".into()), ..Default::default() });
            MENU_HOST.with(|h| *h.borrow_mut() = None);
            let _ = tx.send(InteractiveEvent::Complete(out));
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}

