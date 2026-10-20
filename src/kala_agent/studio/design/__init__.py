"""Design turns: a ConstructionScript compiled and measured by OpenCascade.

The package is intentionally empty on import. The compiler lives in
:mod:`kala_agent.studio.design.compile_ocp` and measurement in
:mod:`kala_agent.studio.design.measure`; importing them requires the optional
``cadquery-ocp`` geometry dependency. Re-exporting here would make every
import of the package require the kernel, which the drawing and journal
modules do not.
"""
