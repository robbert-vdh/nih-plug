//! A base widget for creating other widgets that integrate with NIH-plug's [`Param`] types.

use nih_plug::prelude::*;
use vizia::prelude::*;

use super::RawParamEvent;

/// A helper for creating parameter widgets. The general idea is that a parameter widget struct can
/// adds a `ParamWidgetBase` field on its struct, and then calls [`ParamWidgetBase::view()`] in its
/// view build function. The stored `ParamWidgetbBase` object can then be used in the widget's event
/// handlers to interact with the parameter.
#[derive(Lens)]
pub struct ParamWidgetBase {
    /// We're not allowed to store a reference to the parameter internally, at least not in the
    /// struct that implements [`View`].
    param_ptr: ParamPtr,
}

/// Data and lenses that can be used to draw the parameter widget. The [`param`][Self::param] field
/// should only be used for looking up static data. Prefer the [`make_lens()`][Self::make_lens()]
/// function for binding parameter data to element properties.
pub struct ParamWidgetData<L, Params, P, FMap>
where
    L: Lens<Target = Params> + Clone,
    Params: 'static,
    P: Param + 'static,
    FMap: Fn(&Params) -> &P + Copy + 'static,
{
    // HACK: This needs to be a static reference because of the way bindings in Vizia works. This
    //       feels very wrong, but I don't think there is an alternative. The field is not `pub`
    //       for this reason.
    param: &'static P,
    params: L,
    params_to_param: FMap,
}

impl<L, Params, P, FMap> Clone for ParamWidgetData<L, Params, P, FMap>
where
    L: Lens<Target = Params> + Clone,
    Params: 'static,
    P: Param + 'static,
    FMap: Fn(&Params) -> &P + Copy + 'static,
{
    fn clone(&self) -> Self {
        *self
    }
}

impl<L, Params, P, FMap> Copy for ParamWidgetData<L, Params, P, FMap>
where
    L: Lens<Target = Params> + Copy,
    Params: 'static,
    P: Param + 'static,
    FMap: Fn(&Params) -> &P + Copy + 'static,
{
}

impl<L, Params, P, FMap> ParamWidgetData<L, Params, P, FMap>
where
    L: Lens<Target = Params> + Clone,
    Params: 'static,
    P: Param + 'static,
    FMap: Fn(&Params) -> &P + Copy + 'static,
{
    /// The parameter in question. This can be used for querying static information about the
    /// parameter. Don't use this to get the parameter's current value, use the lenses instead.
    pub fn param(&self) -> &P {
        self.param
    }

    /// Create a lens from a parameter's field. This can be used to bind one of the parameter's
    /// value getters to a property.
    pub fn make_lens<R, F>(&self, f: F) -> impl Lens<Target = R>
    where
        F: Fn(&P) -> R + Clone + 'static,
        R: Clone + 'static,
    {
        let params_to_param = self.params_to_param;

        self.params.map(move |params| {
            let param = params_to_param(params);
            f(param)
        })
    }
}

/// Generate a [`ParamWidgetData`] function that forwards the function call to the underlying
/// `ParamPtr`.
macro_rules! param_ptr_forward(
    (pub fn $method:ident(&self $(, $arg_name:ident: $arg_ty:ty)*) -> $ret:ty) => {
        /// Calls the corresponding method on the underlying [`ParamPtr`] object.
        pub fn $method(&self $(, $arg_name: $arg_ty)*) -> $ret {
            unsafe { self.param_ptr.$method($($arg_name),*) }
        }
    };
);

impl ParamWidgetBase {
    /// Creates a [`ParamWidgetBase`] for the given parameter. This can be stored on a widget object
    /// and used as part of the widget's event handling. To accommodate VIZIA's mapping system,
    /// you'll need to provide a lens containing your `Params` implementation object (check out how
    /// the `Data` struct is used in `gain_gui_vizia`) and a projection function that maps the
    /// `Params` object to the parameter you want to display a widget for. Parameter changes are
    /// handled by emitting [`ParamEvent`][super::ParamEvent]s which are automatically handled by
    /// the VIZIA wrapper.
    pub fn new<L, Params, P, FMap>(cx: &Context, params: L, params_to_param: FMap) -> Self
    where
        L: Lens<Target = Params> + Clone,
        Params: 'static,
        P: Param,
        FMap: Fn(&Params) -> &P + Copy + 'static,
    {
        // We need to do a bit of a nasty and erase the lifetime bound by going through a raw
        // ParamPtr. Vizia requires all lens data to be 'static and Clone.
        let param_ptr = params
            .map(move |params| params_to_param(params).as_ptr())
            .get(cx);

        Self { param_ptr }
    }

    /// Create a view using the a parameter's data. This is not tied to a particular
    /// [`ParamWidgetBase`] instance, but it allows you to easily create lenses for the parameter's
    /// values and access static parameter data.
    pub fn view<L, Params, P, FMap, F, R>(
        cx: &mut Context,
        params: L,
        params_to_param: FMap,
        content: F,
    ) -> R
    where
        L: Lens<Target = Params> + Clone,
        Params: 'static,
        P: Param + 'static,
        FMap: Fn(&Params) -> &P + Copy + 'static,
        F: FnOnce(&mut Context, ParamWidgetData<L, Params, P, FMap>) -> R,
    {
        // We'll provide the raw `&P` to the callbacks to make creating parameter widgets more
        // convenient.
        // SAFETY: This &P won't outlive this function, and in the context of NIH-plug &P will
        //         outlive the editor
        let param: &P = unsafe {
            &*params
                .map(move |params| params_to_param(params) as *const P)
                .get(cx)
        };

        // The widget can use this to access data parameter data and to create lenses for working
        // with the parameter's values
        let param_data = ParamWidgetData {
            param,
            params,
            params_to_param,
        };

        content(cx, param_data)
    }

    /// A shorthand for [`view()`][Self::view()] that can be used directly as an argument to
    /// [`View::build()`].
    pub fn build_view<L, Params, P, FMap, F, R>(
        params: L,
        params_to_param: FMap,
        content: F,
    ) -> impl FnOnce(&mut Context) -> R
    where
        L: Lens<Target = Params> + Clone,
        Params: 'static,
        P: Param + 'static,
        FMap: Fn(&Params) -> &P + Copy + 'static,
        F: FnOnce(&mut Context, ParamWidgetData<L, Params, P, FMap>) -> R,
    {
        move |cx| Self::view(cx, params, params_to_param, content)
    }

    /// Convenience function for using [`ParamWidgetData::make_lens()`]. Whenever possible,
    /// [`view()`][Self::view()] should be used instead.
    pub fn make_lens<L, Params, P, FMap, F, R>(
        params: L,
        params_to_param: FMap,
        f: F,
    ) -> impl Lens<Target = R>
    where
        L: Lens<Target = Params> + Clone,
        Params: 'static,
        P: Param + 'static,
        FMap: Fn(&Params) -> &P + Copy + 'static,
        F: Fn(&P) -> R + Clone + 'static,
        R: Clone + 'static,
    {
        params.map(move |params| {
            let param = params_to_param(params);
            f(param)
        })
    }

    /// Start an automation gesture. This **must** be called before `set_normalized_value()`
    /// is called. Usually this is done on mouse down.
    pub fn begin_set_parameter(&self, cx: &mut EventContext) {
        cx.emit(RawParamEvent::BeginSetParameter(self.param_ptr));
    }

    /// Set the normalized value for a parameter if that would change the parameter's plain value
    /// (to avoid unnecessary duplicate parameter changes). `begin_set_parameter()` **must** be
    /// called before this is called to start an automation gesture, and `end_set_parameter()` must
    /// be called at the end of the gesture.
    pub fn set_normalized_value(&self, cx: &mut EventContext, normalized_value: f32) {
        // This snaps to the nearest plain value if the parameter is stepped in some way.
        // TODO: As an optimization, we could add a `const CONTINUOUS: bool` to the parameter to
        //       avoid this normalized->plain->normalized conversion for parameters that don't need
        //       it
        let plain_value = unsafe { self.param_ptr.preview_plain(normalized_value) };
        let current_plain_value = unsafe { self.param_ptr.unmodulated_plain_value() };
        if plain_value != current_plain_value {
            // For the aforementioned snapping
            let normalized_plain_value = unsafe { self.param_ptr.preview_normalized(plain_value) };
            cx.emit(RawParamEvent::SetParameterNormalized(
                self.param_ptr,
                normalized_plain_value,
            ));
        }
    }

    /// End an automation gesture. This must be called at the end of a gesture, after zero or more
    /// `set_normalized_value()` calls. Usually this is done on mouse down.
    pub fn end_set_parameter(&self, cx: &mut EventContext) {
        cx.emit(RawParamEvent::EndSetParameter(self.param_ptr));
    }

    param_ptr_forward!(pub fn name(&self) -> &str);
    param_ptr_forward!(pub fn unit(&self) -> &'static str);
    param_ptr_forward!(pub fn poly_modulation_id(&self) -> Option<u32>);
    param_ptr_forward!(pub fn modulated_plain_value(&self) -> f32);
    param_ptr_forward!(pub fn unmodulated_plain_value(&self) -> f32);
    param_ptr_forward!(pub fn modulated_normalized_value(&self) -> f32);
    param_ptr_forward!(pub fn unmodulated_normalized_value(&self) -> f32);
    param_ptr_forward!(pub fn default_plain_value(&self) -> f32);
    param_ptr_forward!(pub fn default_normalized_value(&self) -> f32);
    param_ptr_forward!(pub fn step_count(&self) -> Option<usize>);
    param_ptr_forward!(pub fn previous_normalized_step(&self, from: f32, finer: bool) -> f32);
    param_ptr_forward!(pub fn next_normalized_step(&self, from: f32, finer: bool) -> f32);
    param_ptr_forward!(pub fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String);
    param_ptr_forward!(pub fn string_to_normalized_value(&self, string: &str) -> Option<f32>);
    param_ptr_forward!(pub fn preview_normalized(&self, plain: f32) -> f32);
    param_ptr_forward!(pub fn preview_plain(&self, normalized: f32) -> f32);
    param_ptr_forward!(pub fn flags(&self) -> ParamFlags);
}
