# Logical material-state simulation; not a device adapter.
def main(inputs):
    if not inputs['present']:
        raise ValueError('material not detected')
    return {'part': inputs['part'], 'holding': True}
