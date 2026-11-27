**Render side: Blender's denoiser, smooth edges, quiet woods**

With Intel's Open Image Denoise (the denoiser of Blender's Cycles), the Render side denoises with it as the samples settle and denoises the images it saves: one sample through it is already nearer the finished image than four raw ones. The image is anti-aliased as Cycles filters its pixels, the textures are averaged where they are smaller than a pixel, and far leaves and glass add up to how solid they are, so that woods seen from afar settle smooth instead of flickering in a pattern of pixels.
