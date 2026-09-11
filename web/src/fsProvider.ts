import {
  FileChangeType,
  FileSystemProviderCapabilities,
  FileType,
} from "@codingame/monaco-vscode-files-service-override";
import type {
  IFileChange,
  IFileDeleteOptions,
  IFileOverwriteOptions,
  IFileWriteOptions,
  IFileSystemProviderWithFileReadWriteCapability,
  IStat,
  IWatchOptions,
} from "@codingame/monaco-vscode-api/vscode/vs/platform/files/common/files";
import type { URI } from "@codingame/monaco-vscode-api/vscode/vs/base/common/uri";
import { Emitter, Event } from "@codingame/monaco-vscode-api/vscode/vs/base/common/event";
import { api, workspaceUriToPath } from "./api";

type TreeNode = {
  name: string;
  path: string;
  kind: string;
  size?: number;
  children?: TreeNode[];
};

const TREE_TTL_MS = 300;

export class RestFileSystemProvider implements IFileSystemProviderWithFileReadWriteCapability {
  readonly capabilities: FileSystemProviderCapabilities =
    FileSystemProviderCapabilities.FileReadWrite | FileSystemProviderCapabilities.FileFolderCopy;

  private readonly didChangeCapabilities = new Emitter<void>();
  readonly onDidChangeCapabilities: Event<void> = this.didChangeCapabilities.event;

  private readonly didChangeFile = new Emitter<readonly IFileChange[]>();
  readonly onDidChangeFile: Event<readonly IFileChange[]> = this.didChangeFile.event;

  private treeCache: { tree: TreeNode; fetchedAt: number } | undefined;
  private treePromise: Promise<TreeNode> | undefined;

  private fire(type: FileChangeType, resource: URI): void {
    this.treeCache = undefined;
    this.didChangeFile.fire([{ type, resource }]);
  }

  private async getTree(): Promise<TreeNode> {
    if (this.treeCache && Date.now() - this.treeCache.fetchedAt < TREE_TTL_MS) {
      return this.treeCache.tree;
    }
    if (!this.treePromise) {
      this.treePromise = api
        .tree()
        .then((tree) => {
          this.treeCache = { tree, fetchedAt: Date.now() };
          return tree;
        })
        .finally(() => {
          this.treePromise = undefined;
        });
    }
    return this.treePromise;
  }

  watch(_resource: URI, _opts: IWatchOptions): { dispose(): void } {
    return { dispose: () => {} };
  }

  async stat(resource: URI): Promise<IStat> {
    const path = workspaceUriToPath(resource.toString(true));
    const tree = await this.getTree();
    const node = path === "" ? tree : findNode(tree, path);
    if (!node) {
      throw createFileNotFound(resource);
    }
    if (node.kind === "dir") {
      return { type: FileType.Directory, ctime: 0, mtime: 0, size: 0 };
    }
    return { type: FileType.File, ctime: 0, mtime: 0, size: node.size ?? 0 };
  }

  async readFile(resource: URI): Promise<Uint8Array> {
    const path = workspaceUriToPath(resource.toString(true));
    const { content } = await api.readFile(path);
    return new TextEncoder().encode(content);
  }

  async writeFile(resource: URI, content: Uint8Array, _opts: IFileWriteOptions): Promise<void> {
    const path = workspaceUriToPath(resource.toString(true));
    await api.writeFile(path, new TextDecoder().decode(content));
    this.fire(FileChangeType.UPDATED, resource);
  }

  async mkdir(resource: URI): Promise<void> {
    const path = workspaceUriToPath(resource.toString(true));
    await api.mkdir(path);
    this.fire(FileChangeType.ADDED, resource);
  }

  async readdir(resource: URI): Promise<[string, FileType][]> {
    const path = workspaceUriToPath(resource.toString(true));
    const tree = await this.getTree();
    const node = path === "" ? tree : findNode(tree, path);
    if (!node) {
      throw createFileNotFound(resource);
    }
    if (node.kind !== "dir") {
      throw createFileNotFound(resource);
    }
    return (node.children ?? []).map((child): [string, FileType] => [
      child.name,
      child.kind === "dir" ? FileType.Directory : FileType.File,
    ]);
  }

  async delete(resource: URI, _opts: IFileDeleteOptions): Promise<void> {
    const path = workspaceUriToPath(resource.toString(true));
    await api.deleteFile(path);
    this.fire(FileChangeType.DELETED, resource);
  }

  async rename(from: URI, to: URI, _opts: IFileOverwriteOptions): Promise<void> {
    const fromPath = workspaceUriToPath(from.toString(true));
    const toPath = workspaceUriToPath(to.toString(true));
    await api.rename(fromPath, toPath);
    this.fire(FileChangeType.DELETED, from);
    this.fire(FileChangeType.ADDED, to);
  }

  copy(): Promise<void> {
    return Promise.reject(new Error("copy is not supported"));
  }
}

function createFileNotFound(resource: URI): Error {
  const error = new Error(`file not found: ${resource.toString()}`);
  (error as { code?: string }).code = "FileNotFound";
  return error;
}

function findNode(tree: TreeNode, path: string): TreeNode | undefined {
  if (tree.path === path) {
    return tree;
  }
  for (const child of tree.children ?? []) {
    const found = findNode(child, path);
    if (found) return found;
  }
  return undefined;
}
