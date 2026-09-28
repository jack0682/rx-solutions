# Logical material-state simulation; not a device adapter.
def main(inputs):
    if not inputs['clamped']:
        raise ValueError('fixture was not confirmed')
    return {'part': inputs['part'], 'released': True}
