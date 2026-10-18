package app

import jakarta.ws.rs.GET
import jakarta.ws.rs.Path
import io.micronaut.http.annotation.Controller
import io.micronaut.http.annotation.Get

@Path("/r")
class Res {
    @GET
    @Path("/a")
    fun list(): String = ""
}

@Controller("/m")
class MController {
    @Get("/x")
    fun x(): String = ""
}
