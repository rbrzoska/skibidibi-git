import { ComponentFixture, TestBed } from '@angular/core/testing';

import { SkibiBot } from './skibi-bot';

describe('SkibiBot', () => {
  let component: SkibiBot;
  let fixture: ComponentFixture<SkibiBot>;

  beforeEach(async () => {
    await TestBed.configureTestingModule({
      imports: [SkibiBot],
    }).compileComponents();

    fixture = TestBed.createComponent(SkibiBot);
    component = fixture.componentInstance;
    await fixture.whenStable();
  });

  it('should create', () => {
    expect(component).toBeTruthy();
  });

  it('renders the selected facial expression', () => {
    fixture.componentRef.setInput('mood', 'scared');
    fixture.detectChanges();

    expect(fixture.nativeElement.querySelector('.mouth.scared')).not.toBeNull();
    expect(fixture.nativeElement.querySelector('.mouth.happy')).toBeNull();
  });
});
