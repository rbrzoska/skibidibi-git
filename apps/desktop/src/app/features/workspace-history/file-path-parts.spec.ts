import { splitFilePath } from './file-path-parts';

describe('splitFilePath', () => {
  it('keeps the filename separate from a deeply nested Git path', () => {
    expect(splitFilePath('website/front-end-ng/src/app/components/loyalty-card.component.ts')).toEqual({
      directory: 'website/front-end-ng/src/app/components',
      fileName: 'loyalty-card.component.ts',
    });
  });

  it('keeps a root-level filename intact', () => {
    expect(splitFilePath('README.md')).toEqual({ directory: '', fileName: 'README.md' });
  });

  it('handles the separator form returned by Windows-oriented fixtures', () => {
    expect(splitFilePath('src\\app\\main.ts')).toEqual({
      directory: 'src\\app',
      fileName: 'main.ts',
    });
  });
});
