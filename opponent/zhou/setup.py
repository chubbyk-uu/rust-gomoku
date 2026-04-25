from setuptools import Extension, setup

try:
    from Cython.Build import cythonize
except ImportError as exc:  # pragma: no cover - build-time guard
    raise RuntimeError("Cython is required to build native extensions.") from exc


extensions = [
    Extension(
        "gomoku.ai._eval_kernels",
        ["src/gomoku/ai/_eval_kernels.pyx"],
    )
]


setup(
    ext_modules=cythonize(
        extensions,
        compiler_directives={"language_level": "3"},
    )
)
