#!/usr/bin/env python3
"""Compare original PBRT and managed-project exports with a real external r4.
Requires numpy, OpenEXR and Pillow. Does not build or modify pbrt-r4.
"""
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile

import numpy as np
import OpenEXR
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
RENDERER = Path(sys.argv[1]).resolve()


def run(args, cwd):
    result = subprocess.run(args, cwd=cwd, text=True, capture_output=True, timeout=60)
    if result.returncode:
        raise RuntimeError(result.stdout + result.stderr)


def pixels(scene, output, cwd):
    run([str(RENDERER), '--quiet', '--texture-cache', 'off', '--spp', '4',
         '-j', '1', '--outfile', str(output), str(scene)], cwd)
    return OpenEXR.File(str(output)).channels()['RGB'].pixels


with tempfile.TemporaryDirectory(prefix='pui-project-render-') as temporary:
    directory = Path(temporary)
    original = directory / 'original'
    original.mkdir()
    Image.new('RGB', (4, 4), (180, 90, 30)).save(original / 'albedo.png')
    (original / 'floor.ply').write_text('''ply
format ascii 1.0
element vertex 4
property float x
property float y
property float z
element face 2
property list uchar int vertex_indices
end_header
-5 -1 -5
5 -1 -5
5 -1 5
-5 -1 5
3 0 2 1
3 0 3 2
''')
    (original / 'lamp.spd').write_text('400 1\n500 1\n600 1\n700 1\n')
    source = original / 'root.pbrt'
    source.write_text('''LookAt 4 2.5 6 0 0 0 0 1 0
Camera "perspective" "float fov" [42]
Film "rgb" "integer xresolution" [32] "integer yresolution" [32]
Sampler "independent"
Integrator "path" "integer maxdepth" [3]
WorldBegin
LightSource "infinite" "spectrum L" ["lamp.spd"]
Texture "albedo" "spectrum" "imagemap" "string filename" ["albedo.png"]
AttributeBegin
Material "diffuse" "texture reflectance" ["albedo"]
Shape "sphere" "float radius" [1]
AttributeEnd
Material "diffuse" "rgb reflectance" [0.5 0.5 0.5]
Shape "plymesh" "string filename" ["floor.ply"]
''')
    fixtures = [('basic', ROOT / 'tests/fixtures/render-preview.pbrt'), ('assets', source)]
    reports = []
    for name, fixture in fixtures:
        project = directory / f'{name}-project'
        bundle = directory / f'{name}-bundle'
        run(['cargo', 'run', '-p', 'pbrt-io', '--locked', '--example', 'import_bundle',
             '--', str(fixture), str(project), str(bundle)], ROOT)
        before = pixels(fixture, directory / f'{name}-original.exr', fixture.parent)
        if name == 'assets':
            original.rename(directory / 'moved-original')
        after = pixels(bundle / 'scene.pbrt', directory / f'{name}-imported.exr', bundle)
        assert before.shape == after.shape
        assert np.isfinite(before).all() and np.isfinite(after).all()
        equal = bool(np.array_equal(before, after))
        reports.append({'fixture': name, 'size': list(before.shape),
                        'max_abs_difference': float(np.abs(before - after).max()),
                        'equal_pixels': equal})
        assert equal, reports[-1]
    print(json.dumps({'renderer_sha256': hashlib.sha256(RENDERER.read_bytes()).hexdigest(),
                      'spp': 4, 'threads': 1, 'checks': reports}, indent=2))
