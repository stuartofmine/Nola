// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements App logic in the project module.
// - Interfaces:
//   - App
// - APIs:
//   - none
//
import { Component } from 'valdi_core/src/Component';

export class App extends Component {
  onRender() {
    <NavigationRoot>
      {$slot(navigationController => {
        <view width="100%" height="100%" backgroundColor="white">
          <HomeView />
        </view>;
      })}
    </NavigationRoot>;
  }
}

// Note: `HomeView` is provided by `modules/nola_ui`.
