use shared::deleted::UserStats;
use yew::{function_component, html, Html, Properties};

#[derive(Properties, PartialEq)]
pub struct IntroScreenProps {
    pub oauth_link: String,
    pub user_stats: UserStats,
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

#[function_component(IntroScreen)]
pub fn intro_screen(props: &IntroScreenProps) -> Html {
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
                                <span class="stat-label">{"Global Inbox:"}</span>
                                <span class="stat-value">{format_number_with_commas(-(props.user_stats.total_deleted as i64))}</span>
                            </div>
                        </div>
                        <a href={props.oauth_link.clone()} class="signin-button">{"Sign In"}</a>
                    </div>
                </div>
            </div>

            <div class="simple intro-text">
                <h2>{"Welcome to a new concept in email security"}</h2>
                <p>{"Introducing InboxNegative — the email service built for the security-conscious. This RECEIVE ONLY experimental platform delivers your emails only when you're present at the website, ensuring messages live only in the moment. If you're not online, emails are discarded instantly — no backups, no archives, and no trace left behind. Your inbox, permanently tied to your external auth provider, is yours alone."}</p>
                <p>{"Here's why InboxNegative redefines secure communication: emails exist for mere moments in server RAM before they're seamlessly forwarded to your browser tab. After that, they're gone forever. No storage means no records, and without persistent data, there's nothing to hack. It's digital minimalism — offering nearly perfect security through extreme transience."}</p>
                <p>{"We created InboxNegative because we think email is dumb, antiquated, and insecure. We highly recommend that you use it to bypass paywalls, avoid marketing spam, and throw into question the role of an email address as a method for identity or security."}</p>
            </div>

        </div>
    }
}
