import { TestBed } from '@angular/core/testing';

import { CommanderContextStore } from './commander-context';

describe('CommanderContextStore', () => {
  let service: CommanderContextStore;

  beforeEach(() => {
    TestBed.configureTestingModule({});
    service = TestBed.inject(CommanderContextStore);
  });

  it('derives the active workspace context from the route', () => {
    service.updateRoute('/workspace/repo-1/history?commit=abc');
    service.select('commit:abc');

    expect(service.context()).toEqual({
      route: '/workspace/repo-1/history?commit=abc',
      screen: 'history',
      repositoryId: 'repo-1',
      selectedEntity: 'commit:abc',
    });
  });
});
