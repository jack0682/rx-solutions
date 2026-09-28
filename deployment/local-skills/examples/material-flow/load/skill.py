# Logical material-state simulation; not a device adapter.
def main(inputs):
    if not inputs['holding']:
        raise ValueError('material is not held')
    return {'part': inputs['part'], 'clamped': True}
