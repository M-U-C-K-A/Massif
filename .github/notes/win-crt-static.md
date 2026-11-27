**Windows: Massif starts on a fresh Windows**

Massif no longer needs the Visual C++ Redistributable, which a freshly installed Windows does not have: on such a computer Massif could fail to open without a word. The app, massif-cli and the Explorer thumbnails now carry the C runtime inside them, and the few runtime files the denoiser needs come in the zip beside it. A crash before the window appears (in a graphics driver, say) is now said in a message box, with where the start's log is, instead of ending in silence.
