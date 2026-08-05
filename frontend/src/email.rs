use chrono::{DateTime, Utc};
use gloo_timers::callback::Interval;
use shared::email::{Attachment, Email};
use wasm_bindgen::UnwrapThrowExt;
use web_sys::{Element, ShadowRootInit, ShadowRootMode};
use yew::{
    function_component, html, use_effect_with, use_node_ref, use_state, Callback, Html, Properties,
};

#[derive(Properties, PartialEq)]
pub struct Props {
    pub html: String,
}

#[function_component(ShadowDomHtml)]
pub fn shadow_dom_html(props: &Props) -> Html {
    let container_ref = use_node_ref();

    {
        let html = props.html.clone();
        let container_ref = container_ref.clone();

        use_effect_with(html.clone(), move |html| {
            if let Some(container) = container_ref.cast::<Element>() {
                // Create shadow root with "open" mode
                let shadow_root_init = ShadowRootInit::new(ShadowRootMode::Open);
                let shadow_root = container
                    .attach_shadow(&shadow_root_init)
                    .expect_throw("Failed to attach shadow root");

                // Set HTML content to the shadow root
                // Add a style tag to normalize whitespace and fix potential double newlines
                let styled_html = format!(
                    r#"
                        <style>
                            /* Base styling for HTML email content */
                            body, div, p, span, ul, ol, li, h1, h2, h3, h4, h5, h6 {{
                                white-space: normal;
                                line-height: 1.5;
                                margin-top: 0;
                            }}
                            
                            /* Ensure proper paragraph spacing */
                            p {{
                                margin-bottom: 1em;
                            }}
                            
                            /* Improve spacing for lists */
                            ul, ol {{
                                padding-left: 1.5em;
                            }}
                        </style>
                        {}
                    "#,
                    html
                );

                shadow_root.set_inner_html(&styled_html);
            }
            || ()
        });
    }

    html! {
        <div ref={container_ref} class="email-content"></div>
    }
}

#[derive(Clone, PartialEq, Properties)]
pub struct AttachmentItemProps {
    pub attachment: Attachment,
    pub email_time: DateTime<Utc>,
}

#[function_component(AttachmentItem)]
pub fn attachment_item(props: &AttachmentItemProps) -> Html {
    let expiry_time = props.email_time + chrono::Duration::minutes(5);

    let seconds_remaining = use_state(|| {
        let now = Utc::now();
        (expiry_time - now).num_seconds().max(0)
    });

    {
        let seconds_remaining = seconds_remaining.clone();
        use_effect_with((), move |_| {
            let interval = Interval::new(1000, move || {
                let now = Utc::now();
                let remaining = (expiry_time - now).num_seconds().max(0);
                seconds_remaining.set(remaining);
            });

            move || drop(interval)
        });
    }

    let filename = props
        .attachment
        .filename
        .clone()
        .unwrap_or_else(|| "download".to_string());
    let size = props.attachment.size;
    let download_url = format!("/api/attachment/{}", props.attachment.attachment_id);

    // Format size adaptively
    let size_str = if size < 1024 {
        format!("{} bytes", size)
    } else if size < 1024 * 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{:.2} MB", size as f64 / (1024.0 * 1024.0))
    };

    let is_expired = *seconds_remaining == 0;

    // Format time remaining
    let time_str = if is_expired {
        "expired".to_string()
    } else {
        let mins = *seconds_remaining / 60;
        let secs = *seconds_remaining % 60;
        format!("{}:{:02}", mins, secs)
    };

    let item_class = if is_expired {
        "attachment-item expired"
    } else {
        "attachment-item"
    };

    html! {
        <li class={item_class}>
            if is_expired {
                <div class="attachment-link expired">
                    <span class="attachment-icon">{"📎"}</span>
                    <span class="attachment-name">{filename}</span>
                    <span class="attachment-size">{format!("({})", size_str)}</span>
                    <span class="attachment-timer expired">{time_str}</span>
                </div>
            } else {
                <a href={download_url} download={filename.clone()} class="attachment-link">
                    <span class="attachment-icon">{"📎"}</span>
                    <span class="attachment-name">{filename}</span>
                    <span class="attachment-size">{format!("({})", size_str)}</span>
                    <span class="attachment-timer">{time_str}</span>
                </a>
            }
        </li>
    }
}

#[derive(Clone, PartialEq, Properties)]
pub struct EmailProps {
    pub email: Email,
    pub on_delete: Callback<()>,
}

#[function_component(EmailFrame)]
pub fn email_frame(props: &EmailProps) -> Html {
    let on_delete = props.on_delete.clone();
    let handle_delete = Callback::from(move |e: web_sys::MouseEvent| {
        e.prevent_default();
        on_delete.emit(());
    });

    html! {
        <div class="email-frame">
            <div class="email-header">
                <div class="email-header-content">
                    <div class="email-header-text">
                        <div class="email-from">{props.email.from_address()}</div>
                        <div class="email-subject">{props.email.subject()}</div>
                    </div>
                    <button class="email-delete-btn" onclick={handle_delete} title="Delete this email">
                        {"🗑️"}
                    </button>
                </div>
            </div>
            <div class="email-body">
                {
                    match props.email.cleansed_html() {
                        Some(html) => {
                            html! { <ShadowDomHtml html={html} /> }
                        }
                        None => {
                            match props.email.plaintext() {
                                Some(text) => html! { <div class="email-plaintext">{text}</div> },
                                None => html! { <div class="email-raw">{ &props.email.raw }</div> },
                            }
                        }
                    }
                }
            </div>
            {
                if !props.email.attachments.is_empty() {
                    html! {
                        <div class="email-attachments">
                            <h3>{"Attachments"}</h3>
                            <ul class="attachment-list">
                                {
                                    props.email.attachments.iter().map(|attachment| {
                                        html! {
                                            <AttachmentItem
                                                attachment={attachment.clone()}
                                                email_time={props.email.time_of_receipt}
                                            />
                                        }
                                    }).collect::<Html>()
                                }
                            </ul>
                        </div>
                    }
                } else {
                    html! {}
                }
            }
        </div>
    }
}
