// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements HomeView logic in the project module.
// - Interfaces:
//   - HomeView
// - APIs:
//   - none
//
import { Component } from 'valdi_core/src/Component';

export class HomeView extends Component {
  private lastSyncMessage?: string;

  onCreate() {
    this.lastSyncMessage = 'Welcome to Nola (Valdi)';
  }

  onRender() {
    <view padding={12} backgroundColor="#F8F8F8" width="100%" height="100%">
      <label value={this.lastSyncMessage} font={systemFont(18)} />
      <label value={'Today'} marginTop={12} font={systemBoldFont(16)} />
      <label value={'(Summary cards will appear here)'} marginTop={8} />
    </view>;
  }
}
