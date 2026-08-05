use gloo_timers::callback::Timeout;
use yew::{html, Component, Context, Html, Properties};

#[derive(Properties, PartialEq)]
pub struct ToastProps {
    pub message: String,
    pub show: bool,
    pub on_hide: yew::Callback<()>,
}

pub struct Toast {
    _timeout: Option<Timeout>,
}

impl Component for Toast {
    type Message = ();
    type Properties = ToastProps;

    fn create(ctx: &Context<Self>) -> Self {
        let on_hide = ctx.props().on_hide.clone();
        let timeout = if ctx.props().show {
            Some(Timeout::new(3000, move || {
                on_hide.emit(());
            }))
        } else {
            None
        };

        Self { _timeout: timeout }
    }

    fn update(&mut self, _ctx: &Context<Self>, _msg: Self::Message) -> bool {
        false
    }

    fn changed(&mut self, ctx: &Context<Self>, _old_props: &Self::Properties) -> bool {
        if ctx.props().show && self._timeout.is_none() {
            let on_hide = ctx.props().on_hide.clone();
            self._timeout = Some(Timeout::new(3000, move || {
                on_hide.emit(());
            }));
        } else if !ctx.props().show {
            self._timeout = None;
        }
        true
    }

    fn view(&self, ctx: &Context<Self>) -> Html {
        let class = if ctx.props().show {
            "toast show"
        } else {
            "toast"
        };

        html! {
            <div class={class}>
                {&ctx.props().message}
            </div>
        }
    }
}
