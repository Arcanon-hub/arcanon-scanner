This directory contains runtime model files for Arcanon ML refinement.
These files are NOT committed to git — generate them locally using:

    cd ml_research && python fetch_base_model.py

Files generated in models/:
1. codebert-v1.onnx  — Quantized INT8 ONNX weights (~473MB)
2. tokenizer.json    — Byte-level BPE tokenizer config (~3.5MB)

The scanner loads these at runtime from ~/.arcanon/models/ (or a path
configured via ScannerConfig::model_path). They are not embedded in the
binary.
