// Small modal questions: a name to enter, or a yes/no confirmation.

interface PromptRequest {
  kind: 'prompt';
  title: string;
  message?: string;
  value: string;
  placeholder?: string;
  confirm: string;
  resolve: (value: string | null) => void;
}

interface ConfirmRequest {
  kind: 'confirm';
  title: string;
  message?: string;
  confirm: string;
  danger?: boolean;
  resolve: (value: boolean) => void;
}

class Dialogs {
  current = $state<PromptRequest | ConfirmRequest | null>(null);

  prompt(options: { title: string; message?: string; value?: string; placeholder?: string; confirm: string }): Promise<string | null> {
    return new Promise((resolve) => {
      this.current = { kind: 'prompt', value: '', ...options, resolve };
    });
  }

  confirm(options: { title: string; message?: string; confirm: string; danger?: boolean }): Promise<boolean> {
    return new Promise((resolve) => {
      this.current = { kind: 'confirm', ...options, resolve };
    });
  }

  finish(value: string | boolean | null) {
    const request = this.current;
    this.current = null;
    if (!request) return;
    if (request.kind === 'prompt') request.resolve(typeof value === 'string' ? value : null);
    else request.resolve(value === true);
  }
}

export const dialogs = new Dialogs();
