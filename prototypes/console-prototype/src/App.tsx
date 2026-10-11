// PROTOTYPE, throwaway: V1 Console screens and flows (Jbmanllr/automan#80). Run with `npm run dev`.
import { Toaster } from "@/components/ui/sonner"
import { TooltipProvider } from "@/components/ui/tooltip"
import { Console } from "@/components/console/variants"
import { StoreProvider } from "@/store"

export function App() {
  return (
    <TooltipProvider>
      <StoreProvider>
        <Console />
        <Toaster />
      </StoreProvider>
    </TooltipProvider>
  )
}

export default App
