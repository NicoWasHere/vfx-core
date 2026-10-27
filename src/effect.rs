use crate::{Frame, Gpu, ParamSpec, ParamValues};

/// One GPU effect. A host sees only its parameters and the render call.
///
/// Effects are `Send` so hosts that render on several threads (After Effects'
/// Multi-Frame Rendering, for one) can hand a prepared chain from thread to thread.
pub trait Effect: Send {
    fn params(&self) -> &[ParamSpec];

    /// Builds pipelines and internal textures for a format and size.
    /// Called again whenever either changes.
    fn prepare(&mut self, gpu: &Gpu, format: wgpu::TextureFormat, size: (u32, u32));

    /// Renders `input` into `output`. Both frames match the format and size given to `prepare`.
    fn render(&mut self, gpu: &Gpu, input: &Frame, output: &Frame, params: &ParamValues, time: f64);

    /// How many extra frames this effect reads besides `input`, such as a second layer
    /// to blend with. Hosts supply them through [`crate::EffectChain::render_with_layers`].
    fn layer_inputs(&self) -> usize {
        0
    }

    /// Like [`Effect::render`], with the host's extra layers. They match `input` in size
    /// and format. Effects with no layer inputs keep this default and ignore them.
    fn render_layers(
        &mut self,
        gpu: &Gpu,
        input: &Frame,
        _layers: &[&Frame],
        output: &Frame,
        params: &ParamValues,
        time: f64,
    ) {
        self.render(gpu, input, output, params, time);
    }
}
