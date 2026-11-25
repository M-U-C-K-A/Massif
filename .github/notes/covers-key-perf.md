**Smoother on maps of many imported objects**

A map holding many objects imported from WorldPainter (schematics with their own rules) no longer spends several milliseconds of every frame checking whether its layers changed: on Jack's ArdaCraft recording that was 5.8 ms a frame, a third of a frame's time at 60 fps, even with nothing moving.
