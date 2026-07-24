import { TestBed } from '@angular/core/testing';

import { UiFeedback } from './ui-feedback';

describe('UiFeedback', () => {
  let service: UiFeedback;

  beforeEach(() => {
    TestBed.configureTestingModule({});
    service = TestBed.inject(UiFeedback);
  });

  it('keeps the loader active until every scope has finished', () => {
    service.setLoading('router', true, 'Opening view…');
    service.setLoading('workspace', true, 'Reading repository…');

    expect(service.loading()).toBe(true);
    expect(service.loadingLabel()).toBe('Reading repository…');

    service.setLoading('workspace', false);
    expect(service.loading()).toBe(true);
    expect(service.loadingLabel()).toBe('Opening view…');

    service.setLoading('router', false);
    expect(service.loading()).toBe(false);
  });

  it('updates keyed notifications instead of duplicating them', () => {
    service.show('info', 'Fetching', 'First message', { key: 'remote', timeoutMs: 0 });
    service.show('success', 'Done', 'Second message', { key: 'remote', timeoutMs: 0 });

    expect(service.toasts()).toEqual([
      expect.objectContaining({
        kind: 'success',
        title: 'Done',
        message: 'Second message',
      }),
    ]);
  });

  it('notifies the source when a notification is dismissed', () => {
    const onDismiss = vi.fn();
    const id = service.show('error', 'Failed', 'Git failed', { timeoutMs: 0, onDismiss });

    service.dismiss(id);

    expect(service.toasts()).toEqual([]);
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('keeps only the four newest notifications', () => {
    for (let index = 1; index <= 5; index += 1) {
      service.show('info', `Notice ${index}`, `Message ${index}`, { timeoutMs: 0 });
    }

    expect(service.toasts().map(({ title }) => title)).toEqual([
      'Notice 2',
      'Notice 3',
      'Notice 4',
      'Notice 5',
    ]);
  });
});
