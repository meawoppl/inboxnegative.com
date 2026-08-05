use chrono::Datelike;
use yew::{html, Html};

pub fn footer() -> Html {
    html! {
        <footer class="simple footer">
            <div class="copyright">
                <p>{"© "}{chrono::Utc::now().year()}{" InboxNegative - Security Through Transience"}</p>
            </div>
        </footer>
    }
}
