#if !defined(__x86_64__)
#error "boltsnap-xdnd-fix requires x86_64 because Hyprland function hooks are architecture-specific"
#endif

#include <algorithm>
#include <cstdint>
#include <optional>
#include <stdexcept>
#include <string>

#include <src/desktop/view/WLSurface.hpp>
#include <src/helpers/time/Time.hpp>
#include <src/managers/input/InputManager.hpp>
#include <src/plugins/PluginAPI.hpp>
#include <src/xwayland/Dnd.hpp>

namespace {
    CFunctionHook* g_sendEnterHook = nullptr;
    HANDLE         g_handle        = nullptr;

    using SendEnter = void (*)(CX11DataDevice*, uint32_t, SP<CWLSurfaceResource>, const Vector2D&, SP<IDataOffer>);

    [[noreturn]] void fail(const std::string& message) {
        HyprlandAPI::addNotification(g_handle, "[boltsnap-xdnd-fix] " + message, CHyprColor{1.F, 0.2F, 0.2F, 1.F}, 8000.F);
        throw std::runtime_error(message);
    }

    void sendEnterWithInitialPosition(CX11DataDevice* thisptr, uint32_t serial, SP<CWLSurfaceResource> surface, const Vector2D& local, SP<IDataOffer> offer) {
        reinterpret_cast<SendEnter>(g_sendEnterHook->m_original)(thisptr, serial, surface, local, offer);

        const auto hlSurface = Desktop::View::CWLSurface::fromResource(surface);
        const auto box       = hlSurface ? hlSurface->getSurfaceBoxGlobal() : std::nullopt;
        if (!box)
            return;

        const auto position = g_pInputManager->getMouseCoordsInternal() - box->pos();
        thisptr->sendMotion(static_cast<uint32_t>(Time::millis(Time::steadyNow())), position);
    }
} // namespace

APICALL EXPORT std::string PLUGIN_API_VERSION() {
    return HYPRLAND_API_VERSION;
}

APICALL EXPORT PLUGIN_DESCRIPTION_INFO PLUGIN_INIT(HANDLE handle) {
    g_handle = handle;

    const std::string HASH        = __hyprland_api_get_hash();
    const std::string CLIENT_HASH = __hyprland_api_get_client_hash();
    if (HASH != CLIENT_HASH)
        fail("refusing to load: Hyprland ABI mismatch");

    auto METHODS = HyprlandAPI::findFunctionsByName(handle, "sendEnter");
    std::erase_if(METHODS, [](const auto& method) { return !method.demangled.contains("CX11DataDevice::sendEnter("); });
    if (METHODS.size() != 1)
        fail("refusing to load: CX11DataDevice::sendEnter hook was missing or ambiguous");

    g_sendEnterHook = HyprlandAPI::createFunctionHook(handle, METHODS.front().address, reinterpret_cast<void*>(&sendEnterWithInitialPosition));
    if (!g_sendEnterHook || !g_sendEnterHook->hook())
        fail("refusing to load: function hook failed");

    HyprlandAPI::addNotification(handle, "[boltsnap-xdnd-fix] EXPERIMENTAL in-process XDND hook loaded", CHyprColor{1.F, 0.65F, 0.15F, 1.F}, 8000.F);
    return {"boltsnap-xdnd-fix", "Experimental initial XdndPosition workaround", "drvcvt", "0.1.0"};
}

APICALL EXPORT void PLUGIN_EXIT() {
    g_sendEnterHook = nullptr;
    g_handle        = nullptr;
}
