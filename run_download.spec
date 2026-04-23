# -*- mode: python ; coding: utf-8 -*-
from PyInstaller.utils.hooks import collect_all

datas = [('app', 'app')]
binaries = []
hiddenimports = [
    'modelscope',
    'modelscope.hub',
    'modelscope.hub.snapshot_download',
    'modelscope.pipelines',
    'modelscope.outputs',
    'modelscope.utils',
]

Analysis = __import__('PyInstaller.building.build_main', fromlist=['Analysis']).Analysis
EXE = __import__('PyInstaller.building.build_main', fromlist=['EXE']).EXE
PYZ = __import__('PyInstaller.building.build_main', fromlist=['PYZ']).PYZ

a = Analysis(
    ['run_download.py'],
    pathex=[],
    binaries=binaries,
    datas=datas,
    hiddenimports=hiddenimports,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
    optimize=0,
)
pyz = PYZ(a.pure)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.datas,
    [],
    name='run_download_cli',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,
    disable_windowed_traceback=False,
    argv_emulation=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
)