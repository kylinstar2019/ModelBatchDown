import sys

excluded_modules = [
    'torch',
    'torch.nn',
    'torch.nn.functional',
    'torch.utils',
    'torch.cuda',
    'torch.cuda.random',
    'torch.nn.parallel',
    'torch.distributed',
    'torch.multiprocessing',
    'torch.optim',
    'torch.serialization',
    'torch.backends',
    'torch.contrib',
    'torch.for_onnx',
    'torch.onnx',
    'torch.jit',
    'torch.testing',
    'torch.distributions',
    'torch.autograd',
    'torch.tensor',
    'torchvision',
    'torchaudio',
    'scipy',
    'pandas',
    'sklearn',
    'cv2',
    'faiss',
    'faiss_cpu',
    'faiss_gpu',
    'shapely',
    'transformers',
    'peft',
    'accelerate',
    'modelscope.ops',
    'modelscope.ops.ailut',
]

for mod in excluded_modules:
    if mod not in sys.modules:
        class FakeModule:
            def __getattr__(self, name):
                return FakeModule()
            def __call__(self, *args, **kwargs):
                return FakeModule()
        sys.modules[mod] = FakeModule()
