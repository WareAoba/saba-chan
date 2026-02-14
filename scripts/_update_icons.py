from PIL import Image
import os, shutil

root = r'c:\Git\saba-chan'
saba = os.path.join(root, 'resources', 'saba-icon.png')

img = Image.open(saba).convert('RGBA')
ico_sizes = [(16,16),(32,32),(48,48),(64,64),(128,128),(256,256)]

def save_ico(src_img, dest_path):
    """Save a proper multi-size ICO from a source image."""
    src_img.save(dest_path, format='ICO', sizes=ico_sizes)
    size = os.path.getsize(dest_path)
    print('  %s (%d bytes)' % (dest_path, size))
    if size < 5000:
        raise Exception('ICO too small (%d bytes) - generation failed!' % size)

# 1. CLI icon
print('[CLI]')
save_ico(img, os.path.join(root, 'saba-chan-cli', 'icon.ico'))

# 2. Updater icons
print('[Updater]')
upd_icons = os.path.join(root, 'updater', 'gui', 'src-tauri', 'icons')
save_ico(img, os.path.join(upd_icons, 'icon.ico'))
shutil.copy2(saba, os.path.join(upd_icons, 'icon.png'))
print('  icon.png copied')

# 3. GUI build/icon.png + public/icon.png + favicon
print('[GUI]')
for sub in ['build', 'public']:
    d = os.path.join(root, 'saba-chan-gui', sub)
    os.makedirs(d, exist_ok=True)
    shutil.copy2(saba, os.path.join(d, 'icon.png'))
    fav = img.resize((64, 64), Image.LANCZOS)
    fav.save(os.path.join(d, 'favicon.png'), format='PNG')
    print('  %s/icon.png + favicon.png' % sub)

print('\nAll icons updated successfully.')
