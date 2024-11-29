//! An [`Application`] wrapper around an [`IcedEditor`] to bridge between `iced_baseview` and
//! `nih_plug_iced`.

use crossbeam::channel;
use futures_util::FutureExt;
use iced_baseview::{
    baseview::WindowScalePolicy, core::Element, futures::Subscription, window::WindowSubs,
    Renderer, Task,
};
use nih_plug::prelude::GuiContext;
use std::sync::Arc;

use crate::{IcedEditor, ParameterUpdate};

/// Wraps an `iced_baseview` [`Application`] around [`IcedEditor`]. Needed to allow editors to
/// always receive a copy of the GUI context.
pub(crate) struct IcedEditorWrapperApplication<E: IcedEditor> {
    editor: E,

    /// We will receive notifications about parameters being changed on here. Whenever a parameter
    /// update gets sent, we will trigger a [`Message::parameterUpdate`] which causes the UI to be
    /// redrawn.
    parameter_updates_receiver: Arc<channel::Receiver<ParameterUpdate>>,
}

/// This wraps around `E::Message` to add a parameter update message which can be handled directly
/// by this wrapper. That parameter update message simply forces a redraw of the GUI whenever there
/// is a parameter update.
pub enum Message<E: IcedEditor> {
    EditorMessage(E::Message),
    ParameterUpdate,
}

impl<E: IcedEditor> Message<E> {
    fn into_editor_message(self) -> Option<E::Message> {
        if let Message::EditorMessage(message) = self {
            Some(message)
        } else {
            None
        }
    }
}

impl<E: IcedEditor> std::fmt::Debug for Message<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EditorMessage(arg0) => f.debug_tuple("EditorMessage").field(arg0).finish(),
            Self::ParameterUpdate => write!(f, "ParameterUpdate"),
        }
    }
}

impl<E: IcedEditor> Clone for Message<E> {
    fn clone(&self) -> Self {
        match self {
            Self::EditorMessage(arg0) => Self::EditorMessage(arg0.clone()),
            Self::ParameterUpdate => Self::ParameterUpdate,
        }
    }
}

impl<E: IcedEditor> iced_baseview::Application for IcedEditorWrapperApplication<E> {
    type Executor = E::Executor;
    type Message = Message<E>;
    type Flags = (
        Arc<dyn GuiContext>,
        Arc<channel::Receiver<ParameterUpdate>>,
        E::InitializationFlags,
    );
    type Theme = E::Theme;

    fn new(
        (context, parameter_updates_receiver, flags): Self::Flags,
    ) -> (Self, Task<Self::Message>) {
        let (editor, task) = E::new(flags, context);

        (
            Self {
                editor,
                parameter_updates_receiver,
            },
            task.map(Message::EditorMessage),
        )
    }

    #[inline]
    fn update(&mut self, message: Self::Message) -> Task<Self::Message> {
        match message {
            Message::EditorMessage(message) => {
                self.editor.update(message).map(Message::EditorMessage)
            }
            // This message only exists to force a redraw
            Message::ParameterUpdate => Task::none(),
        }
    }

    #[inline]
    fn subscription(
        &self,
        window_subs: &mut WindowSubs<Self::Message>,
    ) -> Subscription<Self::Message> {
        // Since we're wrapping around `E::Message`, we need to do this transformation ourselves
        let on_frame = window_subs.on_frame.clone();
        let on_window_will_close = window_subs.on_window_will_close.clone();
        let mut editor_window_subs: WindowSubs<E::Message> = WindowSubs {
            on_frame: Some(Arc::new(move || {
                let cb = on_frame.clone();
                cb.and_then(|cb| cb().and_then(|m| m.into_editor_message()))
            })),
            on_window_will_close: Some(Arc::new(move || {
                let cb = on_window_will_close.clone();
                cb.and_then(|cb| cb().and_then(|m| m.into_editor_message()))
            })),
        };

        let subscription = Subscription::batch([
            // For some reason there's no adapter to just convert `futures::channel::mpsc::Receiver`
            // into a stream that doesn't require consuming that receiver (which wouldn't work in
            // this case since the subscriptions function gets called repeatedly). So we'll just use
            // a crossbeam queue and this unfold instead.
            Subscription::run_with_id(
                "parameter updates",
                futures_util::stream::unfold(
                    self.parameter_updates_receiver.clone(),
                    |parameter_updates_receiver| match parameter_updates_receiver.try_recv() {
                        Ok(_) => futures_util::future::ready(Some((
                            Message::ParameterUpdate,
                            parameter_updates_receiver,
                        )))
                        .boxed(),
                        Err(channel::TryRecvError::Empty) => {
                            futures_util::future::pending().boxed()
                        }
                        Err(channel::TryRecvError::Disconnected) => {
                            futures_util::future::ready(None).boxed()
                        }
                    },
                ),
            ),
            self.editor
                .subscription(&mut editor_window_subs)
                .map(|m| Message::EditorMessage(m)),
        ]);

        if let Some(message) = editor_window_subs.on_frame.as_ref() {
            let message = Arc::clone(message);
            window_subs.on_frame = Some(Arc::new(move || message().map(Message::EditorMessage)));
        }
        if let Some(message) = editor_window_subs.on_window_will_close.as_ref() {
            let message = Arc::clone(message);
            window_subs.on_window_will_close =
                Some(Arc::new(move || message().map(Message::EditorMessage)));
        }

        subscription
    }

    #[inline]
    fn view(&self) -> Element<'_, Self::Message, Self::Theme, Renderer> {
        self.editor.view().map(Message::EditorMessage)
    }

    #[inline]
    fn scale_policy(&self) -> WindowScalePolicy {
        WindowScalePolicy::SystemScaleFactor
    }

    fn title(&self) -> String {
        self.editor.title()
    }

    fn theme(&self) -> Self::Theme {
        self.editor.theme()
    }
}
