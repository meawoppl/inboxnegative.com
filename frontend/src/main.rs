mod email;
mod footer;
mod intro;
mod toast;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use futures_util::stream::StreamExt;
use gloo::{console::log, net::eventsource::futures::EventSource};
use gloo_net::http::Request;
use gloo_timers::callback::Interval;
use shared::deleted::UserStats;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::spawn_local;
use web_sys::HtmlDocument;
use web_sys::MessageEvent;
use yew::{html, html::Scope, Callback, Component, Context, Html};

use shared::cookies::all_raw;
use shared::email::Email;
use shared::google_oauth;
use shared::jwt::{
    self, unsafe_decode_state_token, InboxNegativeJWT, AUTH_COOKIE_NAME, STATE_COOKIE_NAME,
};

use crate::email::EmailFrame;
use crate::intro::IntroScreen;
use crate::toast::Toast;

fn document() -> HtmlDocument {
    use wasm_bindgen::JsCast;

    web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .dyn_into::<HtmlDocument>()
        .unwrap()
}

pub fn cookie_string() -> String {
    document().cookie().unwrap()
}

fn format_number_with_commas(n: i64) -> String {
    let s = n.to_string();
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    let len = chars.len();

    for (i, ch) in chars.iter().enumerate() {
        result.push(*ch);
        let pos_from_end = len - i - 1;
        if pos_from_end > 0 && pos_from_end.is_multiple_of(3) && *ch != '-' {
            result.push(',');
        }
    }

    result
}

fn unpack_message(event: &MessageEvent) -> Option<Email> {
    match event.data().as_string() {
        Some(msg_data) => match BASE64.decode(msg_data) {
            Ok(decoded) => {
                let json_string = String::from_utf8(decoded).unwrap();
                let deserialized: Email = serde_json::from_str(&json_string).unwrap();

                Some(deserialized)
            }
            Err(e) => {
                log!(format!("Error decoding base64 message: {:?}", e));
                None
            }
        },
        _ => {
            log!("Message event was not a string?!?!");
            None
        }
    }
}

pub async fn get_deleted_stats(link: Scope<App>) -> Result<(), JsValue> {
    // Do the get-request then wait
    let resp = match Request::get("/api/stats").send().await {
        Ok(resp) => resp,
        Err(err) => {
            let msg = format!("Failed to GET deleted stats: {:?}", err);
            return Err(JsValue::from_str(msg.as_str()));
        }
    };

    // Do the deserialization then wait
    let stats = match resp.text().await {
        Ok(text) => serde_json::from_str(&text).unwrap(),
        Err(err) => {
            let msg = format!("Failed to deserialize deleted stats: {:?}", err);
            return Err(JsValue::from_str(msg.as_str()));
        }
    };

    // Poke the frontend with the new stats
    link.send_message(Msg::RenderDeletedCount(stats));

    Ok(())
}

pub async fn read_emails(url: String, link: Scope<App>) -> Result<(), JsValue> {
    let mut es = EventSource::new(&url)
        .map_err(|e| log!(format!("Failed to create EventSource: {:?}", e)))
        .unwrap();

    let mut mail_stream = es.subscribe("mail").unwrap();

    loop {
        log!("Looping");
        let next = mail_stream.next().await;
        match next {
            Some(Ok((event_type, message))) => {
                match unpack_message(&message) {
                    Some(email) => {
                        link.send_message(Msg::AddEmail(email));
                    }
                    None => {
                        log!(format!("{}: {:?}", event_type, message));
                    }
                };
            }
            Some(Err(e)) => {
                log!(format!("Event Source Error: {:?}", e.to_string()));
            }
            None => {
                break;
            }
        }
    }

    log!("EventSource Closed");

    Ok(())
}

// Define the possible messages which can be sent to the component
pub enum Msg {
    AddEmail(Email),
    DeleteEmail(String), // Delete by UUID
    StartStream,
    GetDeletedUpdate,
    RenderDeletedCount(UserStats),
    ShowToast(String),
    HideToast,
}

pub struct App {
    streaming: bool,
    emails: Vec<Email>,
    user_stats: UserStats,
    _stats_update: Interval,
    toast_message: String,
    show_toast: bool,
}

fn copy_to_clipboard(text: String, link: Scope<App>) -> Callback<web_sys::MouseEvent> {
    Callback::from(move |_| {
        let text_to_copy = text.clone();
        let link = link.clone();
        spawn_local(async move {
            if let Some(window) = web_sys::window() {
                let navigator = window.navigator();
                let clipboard = navigator.clipboard();
                match wasm_bindgen_futures::JsFuture::from(clipboard.write_text(&text_to_copy))
                    .await
                {
                    Ok(_) => {
                        link.send_message(Msg::ShowToast(
                            "Email address copied to clipboard!".to_string(),
                        ));
                    }
                    Err(_) => {
                        link.send_message(Msg::ShowToast(
                            "Failed to copy email address".to_string(),
                        ));
                    }
                }
            }
        });
    })
}

fn emails_view(
    emails: &[Email],
    creds: InboxNegativeJWT,
    link: Scope<App>,
    user_stats: &UserStats,
) -> Html {
    html! {
        <div class="container">
            <div class="header">
                <div class="header-content">
                    <div class="header-left">
                        <div class="header-brand">
                            <img src="/assets/logo.svg" alt="InboxNegative Logo" class="header-logo" />
                            <div>
                                <h1>{"InboxNegative"}</h1>
                                <p>{"Security through transience"}</p>
                            </div>
                        </div>
                    </div>
                    <div class="header-right">
                        <div class="header-stats">
                            <div class="stat-item">
                                <span class="stat-label">{"Your Inbox:"}</span>
                                <span class="stat-value">{format_number_with_commas(-(user_stats.user_deleted.unwrap_or(0) as i64))}</span>
                            </div>
                            <div class="stat-item">
                                <span class="stat-label">{"Global:"}</span>
                                <span class="stat-value">{format_number_with_commas(-(user_stats.total_deleted as i64))}</span>
                            </div>
                        </div>
                        <a href="/api/logout" class="logout-button">{"Sign Out"}</a>
                    </div>
                </div>
            </div>

            <div class="simple">
                <h2>{"Your Secure Inbox"}</h2>
                <p>
                    {"Receiving email at: "}
                    <span
                        class="email-copy"
                        style="cursor: pointer; text-decoration: underline; color: #0066cc;"
                        onclick={copy_to_clipboard(creds.email.clone(), link.clone())}
                        title="Click to copy email address"
                    >
                        {&creds.email}
                    </span>
                </p>
            </div>

            <div class="email-container">
                {
                    if emails.is_empty() {
                        html! {
                            <div class="simple">
                                <p>{"No emails yet. They'll appear here when received."}</p>
                            </div>
                        }
                    } else {
                        html! {
                            <>
                                { for emails.iter().rev().map(|email| {
                                    let email_uuid = email.uuid.to_string();
                                    let on_delete = link.callback(move |_| Msg::DeleteEmail(email_uuid.clone()));
                                    html! {
                                        <EmailFrame email={ email.clone() } on_delete={on_delete} />
                                    }
                                }) }
                            </>
                        }
                    }
                }
            </div>
        </div>
    }
}

impl Component for App {
    type Message = Msg;
    type Properties = ();

    fn create(ctx: &Context<Self>) -> Self {
        // Setup the interval to update the counter every 5 seconds
        let link = ctx.link().clone();
        let stats_update = Interval::new(5000, move || {
            // 1000 milliseconds = 1 second
            link.send_message(Msg::GetDeletedUpdate);
        });

        Self {
            streaming: false,
            emails: Vec::new(),
            user_stats: UserStats::new(0, None),
            _stats_update: stats_update,
            toast_message: String::new(),
            show_toast: false,
        }
    }

    fn update(&mut self, ctx: &Context<Self>, msg: Self::Message) -> bool {
        match msg {
            Msg::AddEmail(email) => {
                self.emails.push(email);
                true
            }
            Msg::DeleteEmail(uuid) => {
                self.emails.retain(|e| e.uuid.to_string() != uuid);
                true
            }
            Msg::StartStream => {
                if self.streaming {
                    return false;
                }

                // Start streaming emails
                let email_link = ctx.link().clone();
                spawn_local(async move {
                    read_emails("/api/email".to_string(), email_link)
                        .await
                        .unwrap();
                });

                self.streaming = true;
                false
            }
            Msg::GetDeletedUpdate => {
                let del_link = ctx.link().clone();
                spawn_local(async move {
                    get_deleted_stats(del_link).await.unwrap();
                });
                false
            }
            Msg::RenderDeletedCount(user_stats) => {
                log!(format!(
                    "Updated user stats: {} Total: {}",
                    user_stats.user_deleted.map_or(0, |v| v),
                    user_stats.total_deleted
                ));
                let is_new = self.user_stats != user_stats;
                self.user_stats = user_stats;
                is_new
            }
            Msg::ShowToast(message) => {
                self.toast_message = message;
                self.show_toast = true;
                true
            }
            Msg::HideToast => {
                self.show_toast = false;
                true
            }
        }
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let cookiez = all_raw(&cookie_string());
        let state = cookiez.get(STATE_COOKIE_NAME).unwrap();

        // Extract client_id from state cookie
        let client_id = match unsafe_decode_state_token(state.clone()) {
            Ok(state_token) => state_token.client_id,
            Err(e) => {
                log!(format!("Failed to decode state token: {:?}", e));
                return html! { <div>{"Error loading configuration"}</div> };
            }
        };

        let oauth_link =
            google_oauth::get_oauth_url("https://inboxnegative.com", state, &client_id);

        let auth = cookiez
            .get(AUTH_COOKIE_NAME)
            .map(|cookie| jwt::unsafe_decode_token(cookie.clone()).unwrap());

        ctx.link().send_message(Msg::GetDeletedUpdate);
        html! {
            <>
                {
                    match auth {
                        None => {
                            html! {
                                <IntroScreen oauth_link={oauth_link} user_stats={self.user_stats.clone()}/>
                            }
                        }
                        Some(creds) => {
                            ctx.link().send_message(Msg::StartStream);
                            emails_view(&self.emails, creds, ctx.link().clone(), &self.user_stats)
                        }
                    }
                }
                { footer::footer() }
                <Toast
                    message={self.toast_message.clone()}
                    show={self.show_toast}
                    on_hide={ctx.link().callback(|_| Msg::HideToast)}
                />
            </>
        }
    }
}

fn main() {
    yew::Renderer::<App>::new().render();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_number_with_commas() {
        assert_eq!(format_number_with_commas(0), "0");
        assert_eq!(format_number_with_commas(1), "1");
        assert_eq!(format_number_with_commas(12), "12");
        assert_eq!(format_number_with_commas(123), "123");
        assert_eq!(format_number_with_commas(1234), "1,234");
        assert_eq!(format_number_with_commas(12345), "12,345");
        assert_eq!(format_number_with_commas(123456), "123,456");
        assert_eq!(format_number_with_commas(1234567), "1,234,567");
        assert_eq!(format_number_with_commas(12345678), "12,345,678");
        assert_eq!(format_number_with_commas(123456789), "123,456,789");
        assert_eq!(format_number_with_commas(-1234), "-1,234");
        assert_eq!(format_number_with_commas(-123456), "-123,456");
    }
}
