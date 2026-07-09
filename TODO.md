# TODO

## Refactor right now:
- Beak out GPU upload into module
- Use glam for vector math

## Future:
- Refactor renderer to more cleanly work with more abstracted render passes
  / render graph.
- Break out common logic in glsl shaders to common `#import`:ed files.
- Camera is a mess of recording, player physics, controls, etc.
  Break out when we start implementing player logic properly.
- Clean up asset tools when they are closer to complete.
