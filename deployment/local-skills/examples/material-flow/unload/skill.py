# Logical material-state simulation; not a device adapter.
def main(inputs):
    if not inputs['released']:
        raise ValueError('fixture has not released the material')
    return {'part': inputs['part'], 'location': 'output-bin'}
