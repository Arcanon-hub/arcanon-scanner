import os
from optimum.onnxruntime import ORTModelForFeatureExtraction
from transformers import AutoTokenizer
import shutil

def fetch_model():
    model_id = "microsoft/codebert-base"
    save_dir = "temp_model"
    
    print(f"Fetching {model_id} and exporting to ONNX...")
    # Export to ONNX
    # Note: ORTModelForFeatureExtraction doesn't typically use dtype directly in from_pretrained for export
    model = ORTModelForFeatureExtraction.from_pretrained(model_id, export=True)
    tokenizer = AutoTokenizer.from_pretrained(model_id)
    
    os.makedirs("models", exist_ok=True)
    
    # Save the tokenizer
    tokenizer.save_pretrained(save_dir)
    if os.path.exists(os.path.join(save_dir, "tokenizer.json")):
        shutil.copy(os.path.join(save_dir, "tokenizer.json"), "models/tokenizer.json")
    
    # Export and save the model
    model.save_pretrained(save_dir)
    if os.path.exists(os.path.join(save_dir, "model.onnx")):
        shutil.copy(os.path.join(save_dir, "model.onnx"), "models/codebert-v1.onnx")
    
    print("\nSuccess!")
    print("Files created in models/: codebert-v1.onnx, tokenizer.json")

if __name__ == "__main__":
    fetch_model()
