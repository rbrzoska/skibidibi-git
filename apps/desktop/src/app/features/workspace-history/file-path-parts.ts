export interface FilePathParts {
  readonly directory: string;
  readonly fileName: string;
}

/** Splits a Git display path without normalizing or decoding its contents. */
export function splitFilePath(path: string): FilePathParts {
  const separatorIndex = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'));
  if (separatorIndex < 0) {
    return { directory: '', fileName: path };
  }
  return {
    directory: path.slice(0, separatorIndex),
    fileName: path.slice(separatorIndex + 1),
  };
}
