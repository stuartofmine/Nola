package com.nola.ui.navigation

import com.nola.ui.components.dismissImeOnBlankTap
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.border
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.displayCutoutPadding
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.WbSunny
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Notifications
import androidx.compose.material.icons.filled.Memory
import androidx.compose.material.icons.filled.QuestionAnswer
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.SmartToy
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.Alignment
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import com.nola.ui.screen.chat.ChatScreen
import com.nola.ui.screen.home.TodayScreen
import com.nola.ui.screen.home.DataWorkspaceScreen
import com.nola.ui.screen.home.InformationOverview
import com.nola.ui.screen.intelligence.IntelligenceScreen
import com.nola.ui.screen.intelligence.IntelligenceSection
import com.nola.ui.screen.models.ModelsScreen
import com.nola.ui.components.rememberNolaHaptics
import com.nola.ui.screen.settings.SettingsScreen
import com.nola.ui.components.NolaColors
import com.nola.ui.components.rememberReduceMotionEnabled
import com.nola.ui.components.NolaMotion
import com.nola.ui.components.nolaTween
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import com.nola.R

enum class NolaTab(val route: String, val labelRes: Int) {
    Chat("chat", R.string.tab_chat),
    Home("home", R.string.today_title),
    Activity("activity", R.string.tab_activity),
    Information("information", R.string.tab_information),
    Intelligence("intelligence", R.string.tab_intelligence),
    Models("models", R.string.tab_models),
    Settings("settings", R.string.tab_settings)
}

private const val SETTINGS_EXPERTS_ROUTE = "settings/experts"
private const val MODELS_ROUTE = "models"
private const val WORKSPACE_ROUTE = "workspace"

internal enum class NolaNavigationLayout {
    Bottom,
    CompactSide,
    ExpandedSide
}

internal fun nolaNavigationLayout(widthDp: Float): NolaNavigationLayout = when {
    widthDp < 680f -> NolaNavigationLayout.Bottom
    widthDp < 1180f -> NolaNavigationLayout.CompactSide
    else -> NolaNavigationLayout.ExpandedSide
}

@Composable
fun NolaScaffold(
    modifier: Modifier = Modifier,
    openModelsAfterWelcome: Boolean = false,
    onWelcomeDestinationOpened: () -> Unit = {},
) {
    val navController = rememberNavController()
    var chatDetailVisible by rememberSaveable { mutableStateOf(false) }
    val navBackStackEntry by navController.currentBackStackEntryAsState()
    val currentRoute = navBackStackEntry?.destination?.route
    LaunchedEffect(openModelsAfterWelcome, currentRoute) {
        if (openModelsAfterWelcome && currentRoute != null) {
            navController.navigate(NolaTab.Models.route) { launchSingleTop = true }
            onWelcomeDestinationOpened()
        }
    }
    val selectedTab = when {
        currentRoute?.startsWith(MODELS_ROUTE) == true || currentRoute == NolaTab.Intelligence.route || currentRoute == SETTINGS_EXPERTS_ROUTE -> NolaTab.Settings
        currentRoute == WORKSPACE_ROUTE -> NolaTab.entries.firstOrNull {
            it.route == navController.previousBackStackEntry?.destination?.route
        } ?: NolaTab.Information
        else -> NolaTab.entries.firstOrNull { it.route == currentRoute } ?: NolaTab.Chat
    }

    val navigateToTab: (NolaTab) -> Unit = { tab ->
        val reselected = tab == selectedTab
        navController.navigate(tab.route) {
            popUpTo(navController.graph.startDestinationId) {
                saveState = true
            }
            launchSingleTop = true
            restoreState = true
        }
        // Preserve restored editors when switching tabs. Only reselecting the
        // current tab returns to its root.
        if (reselected) navController.popBackStack(tab.route, inclusive = false)
    }

    BoxWithConstraints(modifier = modifier.fillMaxSize().dismissImeOnBlankTap()) {
        val navigationLayout = nolaNavigationLayout(maxWidth.value)
        val usesSideNavigation = navigationLayout != NolaNavigationLayout.Bottom
        val usesExpandedSideNavigation = navigationLayout == NolaNavigationLayout.ExpandedSide

        val showBottomBar = !usesSideNavigation && !(currentRoute == NolaTab.Chat.route && chatDetailVisible)
        val reduceMotion = rememberReduceMotionEnabled()

        Scaffold(
            modifier = Modifier.fillMaxSize(),
            containerColor = MaterialTheme.colorScheme.background,
            contentWindowInsets = if (usesSideNavigation) WindowInsets(0) else androidx.compose.material3.ScaffoldDefaults.contentWindowInsets,
            bottomBar = {
                AnimatedVisibility(
                    visible = showBottomBar,
                    enter = slideInVertically(
                        animationSpec = nolaTween(NolaMotion.base, reduceMotion),
                        initialOffsetY = { it },
                    ) + fadeIn(nolaTween(NolaMotion.fast, reduceMotion)),
                    exit = slideOutVertically(
                        animationSpec = nolaTween(NolaMotion.fast, reduceMotion),
                        targetOffsetY = { it },
                    ) + fadeOut(nolaTween(NolaMotion.fast, reduceMotion)),
                ) {
                    NolaBottomNavigation(
                        selectedTab = selectedTab,
                        onSelect = navigateToTab
                    )
                }
            }
        ) { padding ->
            Row(
                modifier = Modifier
                    .fillMaxSize()
                    .then(
                        if (usesSideNavigation) Modifier
                        else Modifier.padding(padding).consumeWindowInsets(padding)
                    )
            ) {
                if (usesSideNavigation) {
                    NolaWideNavigation(
                        selectedTab = selectedTab,
                        onSelect = navigateToTab,
                        expanded = usesExpandedSideNavigation
                    )
                }

                NolaNavigationHost(
                    navController = navController,
                    onChatDetailChanged = { chatDetailVisible = it },
                    compact = !usesSideNavigation,
                    modifier = Modifier
                        .weight(1f)
                        .then(if (usesSideNavigation) Modifier.statusBarsPadding().navigationBarsPadding().consumeWindowInsets(WindowInsets.navigationBars).displayCutoutPadding() else Modifier)
                )
            }
        }
    }
}

@Composable
internal fun NolaBottomNavigation(
    selectedTab: NolaTab,
    onSelect: (NolaTab) -> Unit
) {
    val haptics = rememberNolaHaptics()
    val reduceMotion = rememberReduceMotionEnabled()
    val largeLabels = LocalDensity.current.fontScale > 1.3f
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.background)
    ) {
        HorizontalDivider(
            thickness = 0.5.dp,
            color = NolaColors.hairlineBorder
        )
        if (largeLabels) {
            // The full selected destination remains visible without shrinking accessibility text.
            // Each tab already exposes its complete name to accessibility services.
            Text(
                text = stringResource(selectedTab.labelRes),
                style = MaterialTheme.typography.labelLarge,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)
                    .testTag("navigation.currentDestination").clearAndSetSemantics {},
                textAlign = androidx.compose.ui.text.style.TextAlign.Center
            )
        }
        Row(
            modifier = Modifier.fillMaxWidth().navigationBarsPadding()
                .padding(horizontal = if (LocalConfiguration.current.screenWidthDp < 360) 8.dp else 12.dp, vertical = 5.dp)
                .testTag("navigation.bottom"),
            horizontalArrangement = Arrangement.spacedBy(4.dp)
        ) {
            listOf(NolaTab.Chat, NolaTab.Activity, NolaTab.Information, NolaTab.Settings).forEach { tab ->
                val tabLabel = stringResource(tab.labelRes)
                val compactLabel = tabLabel
                val selected = selectedTab == tab
                val containerColor by animateColorAsState(
                    targetValue = if (selected) NolaColors.navSelectionPill else Color.Transparent,
                    animationSpec = nolaTween(NolaMotion.fast, reduceMotion),
                    label = "bottom-nav-container",
                )
                val tint by animateColorAsState(
                    targetValue = if (selected) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                    animationSpec = nolaTween(NolaMotion.fast, reduceMotion),
                    label = "bottom-nav-tint",
                )
                Column(
                    modifier = Modifier.weight(1f).heightIn(min = 52.dp)
                        .clip(RoundedCornerShape(8.dp))
                        .background(containerColor)
                        .selectable(selected = selected, role = Role.Tab, onClick = {
                            if (!selected) haptics.selection()
                            onSelect(tab)
                        }).semantics { contentDescription = tabLabel }
                        .padding(horizontal = 2.dp, vertical = 5.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(4.dp, Alignment.CenterVertically)
                ) {
                    NolaTabIcon(tab, contentDescription = null, tint = tint, modifier = Modifier.size(20.dp))
                    if (!largeLabels) Text(compactLabel, maxLines = 1, overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.labelSmall.copy(fontSize = 12.sp, lineHeight = 16.sp), color = tint,
                        fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal)
                }
            }
        }
    }
}

@Composable
private fun NolaWideNavigation(
    selectedTab: NolaTab,
    onSelect: (NolaTab) -> Unit,
    expanded: Boolean
) {
    val haptics = rememberNolaHaptics()
    val reduceMotion = com.nola.ui.components.rememberReduceMotionEnabled()
    val navigationWidth by animateDpAsState(
        targetValue = if (expanded) 216.dp else 76.dp,
        animationSpec = tween(
            durationMillis = if (reduceMotion) 0 else NolaMotion.base,
            easing = NolaMotion.emphasizedEasing
        ),
        label = "sideNavigationWidth"
    )
    Row(modifier = Modifier.fillMaxHeight()) {
        Surface(
            modifier = Modifier
                .fillMaxHeight()
                .width(navigationWidth)
                .testTag(if (expanded) "navigation.side.expanded" else "navigation.side.compact"),
            color = MaterialTheme.colorScheme.surface.copy(alpha = 0.98f),
            tonalElevation = 0.dp
        ) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .statusBarsPadding()
                    .navigationBarsPadding()
                    .padding(horizontal = 8.dp, vertical = 14.dp),
                horizontalAlignment = Alignment.CenterHorizontally
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxWidth()
                        .height(48.dp)
                        .padding(horizontal = if (expanded) 10.dp else 0.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = if (expanded) Arrangement.Start else Arrangement.Center
                ) {
                    Box(
                        modifier = Modifier
                            .size(34.dp)
                            .clip(RoundedCornerShape(11.dp))
                            .background(MaterialTheme.colorScheme.primary),
                        contentAlignment = Alignment.Center
                    ) {
                        Text(
                            text = "n",
                            color = MaterialTheme.colorScheme.onPrimary,
                            fontWeight = FontWeight.Bold
                        )
                    }
                    AnimatedVisibility(
                        visible = expanded,
                        enter = fadeIn(tween(if (reduceMotion) 0 else NolaMotion.base, easing = NolaMotion.emphasizedEasing)),
                        exit = fadeOut(tween(if (reduceMotion) 0 else NolaMotion.fast))
                    ) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                        Spacer(Modifier.width(12.dp))
                        Text(
                            text = "Nola",
                            style = MaterialTheme.typography.titleMedium,
                            fontWeight = FontWeight.SemiBold
                        )
                        }
                    }
                }

                Spacer(Modifier.height(14.dp))
                listOf(NolaTab.Chat, NolaTab.Activity, NolaTab.Information).forEach { tab ->
                    NolaSideNavigationItem(
                        tab = tab,
                        selected = selectedTab == tab,
                        expanded = expanded,
                        onClick = {
                            if (selectedTab != tab) haptics.selection()
                            onSelect(tab)
                        }
                    )
                }

                Spacer(Modifier.weight(1f))
                HorizontalDivider(
                    modifier = Modifier.padding(horizontal = 10.dp, vertical = 8.dp),
                    thickness = 0.5.dp,
                    color = NolaColors.hairlineBorder
                )
                NolaSideNavigationItem(
                    tab = NolaTab.Settings,
                    selected = selectedTab == NolaTab.Settings,
                    expanded = expanded,
                    onClick = {
                        if (selectedTab != NolaTab.Settings) haptics.selection()
                        onSelect(NolaTab.Settings)
                    }
                )
            }
        }
        VerticalDivider(
            modifier = Modifier.fillMaxHeight(),
            thickness = 0.5.dp,
            color = NolaColors.hairlineBorder
        )
    }
}

@Composable
private fun NolaSideNavigationItem(
    tab: NolaTab,
    selected: Boolean,
    expanded: Boolean,
    onClick: () -> Unit
) {
    val reduceMotion = rememberReduceMotionEnabled()
    val selectionBackground by animateColorAsState(
        targetValue = if (selected) NolaColors.sideNavSelectionPill else Color.Transparent,
        animationSpec = tween(durationMillis = if (reduceMotion) 0 else NolaMotion.fast),
        label = "sideNavigationBackground"
    )
    val contentColor = if (selected) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 3.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(selectionBackground, RoundedCornerShape(12.dp))
            .selectable(selected = selected, role = Role.Tab, onClick = onClick)
            .heightIn(min = 48.dp)
            .padding(horizontal = if (expanded) 14.dp else 0.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = if (expanded) Arrangement.Start else Arrangement.Center
    ) {
        NolaTabIcon(tab, contentDescription = if (expanded) null else stringResource(tab.labelRes), tint = contentColor)
        AnimatedVisibility(
            visible = expanded,
            enter = fadeIn(tween(if (reduceMotion) 0 else NolaMotion.base, easing = NolaMotion.emphasizedEasing)),
            exit = fadeOut(tween(if (reduceMotion) 0 else NolaMotion.fast))
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Spacer(Modifier.width(14.dp))
                Text(
                    text = stringResource(tab.labelRes),
                    color = contentColor,
                    style = MaterialTheme.typography.labelLarge,
                    fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis
                )
            }
        }
    }
}

@Composable
private fun NolaTabIcon(
    tab: NolaTab,
    contentDescription: String?,
    tint: androidx.compose.ui.graphics.Color? = null,
    modifier: Modifier = Modifier
) {
    val resolvedTint = tint ?: LocalContentColor.current
    when (tab) {
        NolaTab.Chat -> Icon(painterResource(R.drawable.nola_chat), contentDescription, tint = resolvedTint, modifier = modifier)
        NolaTab.Activity -> Icon(Icons.Outlined.Notifications, contentDescription, tint = resolvedTint, modifier = modifier)
        NolaTab.Home -> Icon(painterResource(R.drawable.nola_sun), contentDescription, tint = resolvedTint, modifier = modifier)
        NolaTab.Information -> Icon(painterResource(R.drawable.nola_folder), contentDescription, tint = resolvedTint, modifier = modifier)
        NolaTab.Intelligence -> Icon(Icons.Filled.Memory, contentDescription, tint = resolvedTint, modifier = modifier)
        NolaTab.Models -> Icon(painterResource(R.drawable.nola_models), contentDescription, tint = resolvedTint, modifier = modifier)
        NolaTab.Settings -> Icon(painterResource(R.drawable.nola_settings), contentDescription, tint = resolvedTint, modifier = modifier)
    }
}

@Composable
private fun NolaNavigationHost(
    navController: androidx.navigation.NavHostController,
    onChatDetailChanged: (Boolean) -> Unit,
    compact: Boolean,
    modifier: Modifier = Modifier
) {
    var workspacePage by rememberSaveable { mutableStateOf(0) }
    var requestedConversationId by rememberSaveable { mutableStateOf<String?>(null) }
    var requestedNewConversation by rememberSaveable { mutableStateOf(false) }
    fun openChat(owner: String? = null, newConversation: Boolean = false) {
        requestedConversationId = owner
        requestedNewConversation = newConversation
        navController.navigate(NolaTab.Chat.route) {
            popUpTo(navController.graph.startDestinationId) { saveState = true }
            launchSingleTop = true
            restoreState = true
        }
    }
    val reduceMotion = rememberReduceMotionEnabled()
    NavHost(
        navController = navController,
        startDestination = NolaTab.Chat.route,
        modifier = modifier,
        enterTransition = { fadeIn(tween(if (reduceMotion) 0 else NolaMotion.controlState)) },
        exitTransition = { fadeOut(tween(if (reduceMotion) 0 else NolaMotion.fast)) },
        popEnterTransition = { fadeIn(tween(if (reduceMotion) 0 else NolaMotion.controlState)) },
        popExitTransition = { fadeOut(tween(if (reduceMotion) 0 else NolaMotion.fast)) },
    ) {
            composable(NolaTab.Chat.route) {
                ChatScreen(
                    requestedConversationId = requestedConversationId,
                    requestedNewConversation = requestedNewConversation,
                    onRequestedConversationConsumed = {
                        requestedConversationId = null
                        requestedNewConversation = false
                    },
                    onDetailVisibilityChanged = onChatDetailChanged,
                    onOpenPlans = { workspacePage = 2; navController.navigate(WORKSPACE_ROUTE) { launchSingleTop = true } },
                    onOpenModels = {
                        navController.navigate(NolaTab.Models.route) {
                            launchSingleTop = true
                        }
                    }
                )
            }
            composable(NolaTab.Activity.route) {
                com.nola.ui.screen.activity.ActivityScreen(onOpenPlans = { workspacePage = 2; navController.navigate(WORKSPACE_ROUTE) { launchSingleTop = true } }, onOpenConversation = { owner ->
                    openChat(owner)
                })
            }
            composable(NolaTab.Home.route) {
                TodayScreen(onOpenWorkspace = { page ->
                    workspacePage = page
                    navController.navigate(WORKSPACE_ROUTE) { launchSingleTop = true }
                }, onOpenConversations = { openChat() },
                    onOpenConversation = { owner ->
                        openChat(owner)
                    })
            }
            composable(NolaTab.Information.route) {
                if (compact) InformationOverview(onOpen = { page ->
                    workspacePage = page
                    navController.navigate(WORKSPACE_ROUTE) { launchSingleTop = true }
                }) else DataWorkspaceScreen(initialPage = 4, onCreateInChat = {
                    openChat(newConversation = true)
                }, onOpenConversation = { owner ->
                    openChat(owner)
                })
            }
            composable(WORKSPACE_ROUTE) {
                val returnFromWorkspace: () -> Unit = {
                    if (!navController.popBackStack()) {
                        navController.navigate(NolaTab.Information.route) { launchSingleTop = true }
                    }
                }
                BackHandler(enabled = compact, onBack = returnFromWorkspace)
                DataWorkspaceScreen(initialPage = workspacePage, showSwitcher = !compact,
                    backToInformation = compact && navController.previousBackStackEntry?.destination?.route == NolaTab.Information.route,
                    onBack = returnFromWorkspace,
                    onOpenConversation = { owner ->
                        openChat(owner)
                    }, onCreateInChat = {
                    openChat(newConversation = true)
                })
            }
            composable(NolaTab.Intelligence.route) {
                val fromSettings = navController.previousBackStackEntry?.destination?.route == NolaTab.Settings.route
                IntelligenceScreen(
                    section = IntelligenceSection.EXTENSIONS,
                    onBack = if (fromSettings) ({ navController.popBackStack() }) else null,
                )
            }
            composable(NolaTab.Models.route) {
                val hasPrevious = navController.previousBackStackEntry != null
                ModelsScreen(onBack = if (hasPrevious) ({ navController.popBackStack() }) else null)
            }
            composable(SETTINGS_EXPERTS_ROUTE) { entry ->
                val chatEntry = androidx.compose.runtime.remember(entry) { navController.getBackStackEntry(NolaTab.Chat.route) }
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) {
                    Box(Modifier.widthIn(max = 720.dp).fillMaxWidth()) {
                        com.nola.ui.screen.chat.ExpertManagementPanel(
                            viewModel = androidx.hilt.navigation.compose.hiltViewModel(chatEntry),
                            localDirectory = true,
                            onBack = { navController.popBackStack() },
                        )
                    }
                }
            }
            composable(NolaTab.Settings.route) {
                SettingsScreen(
                    onOpenConversation = { openChat(it) },
                    onOpenDataSources = {
                        workspacePage = 0
                        navController.navigate(WORKSPACE_ROUTE) {
                            launchSingleTop = true
                        }
                    },
                    onOpenModels = {
                        navController.navigate(NolaTab.Models.route) { launchSingleTop = true }
                    },
                    onOpenExperts = { navController.navigate(SETTINGS_EXPERTS_ROUTE) { launchSingleTop = true } },
                    onOpenIntelligence = {
                        navController.navigate(NolaTab.Intelligence.route) { launchSingleTop = true }
                    },
                    onOpenChat = {
                        openChat(newConversation = true)
                    }
                )
            }
    }
}
